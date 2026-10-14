mod audio;
mod cleanup;
mod local;
mod logger;
mod openrouter;
mod paste;
mod settings;
mod status;
mod transcribe;
mod usage;

use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    ActivationPolicy, AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use settings::{AppState, Engine, Settings};
use status::Status;
use usage::{Totals, UsageStore};

/// Clips shorter than this are treated as accidental taps.
const MIN_CLIP_SECS: f32 = 0.3;
/// Clips whose loudest sample is below this are treated as silence.
const SILENCE_PEAK: f32 = 0.005;
/// How long the error icon stays up before returning to idle.
const ERROR_FLASH: Duration = Duration::from_secs(3);

enum Outcome {
    Pasted,
    Discarded,
    Failed,
}

fn on_key_down(app: &AppHandle) {
    let state = app.state::<AppState>();
    let device = state.settings.lock().unwrap().input_device.clone();
    let mut slot = state.recording.lock().unwrap();
    if slot.is_some() {
        return; // auto-repeat or double press
    }
    match audio::start(device) {
        Ok(rec) => {
            *slot = Some(rec);
            status::set(app, Status::Listening);
            log::info!("recording started");
        }
        Err(e) => {
            log::error!("could not start recording: {e}");
            flash_error(app);
        }
    }
}

fn on_key_up(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(active) = state.recording.lock().unwrap().take() else {
        return;
    };
    let released_at = Instant::now();
    let (model, engine, local_model) = {
        let s = state.settings.lock().unwrap();
        (s.cleanup_model.clone(), s.engine, s.local_model.clone())
    };
    let job = Job { model, engine, local_model };
    status::set(app, Status::Processing);
    match active.stop() {
        Ok(recording) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match process(&app, recording, job, released_at).await {
                    Outcome::Pasted | Outcome::Discarded => status::set(&app, Status::Idle),
                    Outcome::Failed => flash_error(&app),
                }
            });
        }
        Err(e) => {
            log::error!("could not stop recording: {e}");
            flash_error(app);
        }
    }
}

/// Show the error icon briefly, then return to idle.
fn flash_error(app: &AppHandle) {
    status::set(app, Status::Error);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = tauri::async_runtime::spawn_blocking(|| std::thread::sleep(ERROR_FLASH)).await;
        // Don't stomp a recording that started during the flash.
        if app.state::<AppState>().recording.lock().unwrap().is_none() {
            status::set(&app, Status::Idle);
        }
    });
}

/// Settings captured when the key was released, so a change in the settings
/// window cannot affect a dictation that is already in flight.
struct Job {
    /// Clean-up model.
    model: String,
    engine: Engine,
    local_model: String,
}

fn word_count(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

async fn process(
    app: &AppHandle,
    recording: audio::Recording,
    job: Job,
    released_at: Instant,
) -> Outcome {
    let Job { model, engine, local_model } = job;
    let secs = recording.duration_secs();
    log::info!(
        "captured {secs:.2}s of audio (peak {:.3}, rms {:.4})",
        recording.peak(),
        recording.rms()
    );
    if secs < MIN_CLIP_SECS {
        log::info!("clip too short, discarded");
        return Outcome::Discarded;
    }
    if recording.peak() < SILENCE_PEAK {
        log::info!("clip silent, discarded");
        return Outcome::Discarded;
    }

    let started = Instant::now();
    let (result, engine_label) = match engine {
        Engine::Cloud => {
            let result = match recording.to_wav() {
                Ok(wav) => {
                    // Debug builds only: keep the last clip so capture quality can
                    // be checked by ear. Release builds never write audio to disk.
                    if cfg!(debug_assertions) {
                        let path = std::env::temp_dir().join("dictation-ai-last.wav");
                        if std::fs::write(&path, &wav).is_ok() {
                            log::debug!("saved {} ({} KB)", path.display(), wav.len() / 1024);
                        }
                    }
                    transcribe::transcribe(&wav).await
                }
                Err(e) => Err(format!("WAV encode failed: {e}")),
            };
            (result, "cloud".to_string())
        }
        Engine::Local => (
            local::transcribe(app, &local_model, recording.samples).await,
            format!("local {local_model}"),
        ),
    };

    // No fallback between engines: a failure is reported, and the user can
    // switch engines in settings.
    let transcription = match result {
        Ok(t) if t.text.is_empty() => {
            log::info!("transcription was empty, nothing to paste");
            return Outcome::Discarded;
        }
        Ok(t) => t,
        Err(e) => {
            log::error!("transcription failed ({engine_label}): {e}");
            return Outcome::Failed;
        }
    };
    log::info!(
        "transcribed {} words ({engine_label}) in {} ms, {} ms after release (cost ${:.5})",
        word_count(&transcription.text),
        started.elapsed().as_millis(),
        released_at.elapsed().as_millis(),
        transcription.cost_usd
    );
    log::debug!("transcript: {:?}", transcription.text);

    let (cleaned, cleanup_cost) = cleanup::clean(&transcription.text, &model).await;
    let (final_text, cleanup_words) = match cleaned {
        Ok(text) => {
            log::info!(
                "cleaned up with {model} at {} ms (cost ${cleanup_cost:.5})",
                released_at.elapsed().as_millis()
            );
            log::debug!("cleaned: {text:?}");
            let words = word_count(&text);
            (text, words)
        }
        Err(e) => {
            log::warn!("clean-up failed with {model}, using raw transcript: {e}");
            (transcription.text.clone(), 0)
        }
    };

    // Cost has been incurred by now, so tally it even if the paste fails.
    app.state::<AppState>().usage.record(Totals {
        audio_seconds: f64::from(secs),
        transcription_words: word_count(&transcription.text),
        transcription_cost_usd: transcription.cost_usd,
        cleanup_words,
        cleanup_cost_usd: cleanup_cost,
    });

    match tauri::async_runtime::spawn_blocking(move || paste::paste_text(&final_text)).await {
        Ok(Ok(())) => {
            log::info!("pasted, {} ms after release", released_at.elapsed().as_millis());
            Outcome::Pasted
        }
        Ok(Err(e)) => {
            log::error!("paste failed: {e}");
            Outcome::Failed
        }
        Err(e) => {
            log::error!("paste task failed: {e}");
            Outcome::Failed
        }
    }
}

/// Show the window with this label, creating it on first use. Closing a
/// window destroys it, so it is rebuilt the next time it is opened.
fn open_window(
    app: &AppHandle,
    label: &str,
    page: &str,
    title: &str,
    size: (f64, f64),
) -> Option<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("Dictation AI Settings")
        .inner_size(480.0, 680.0)
        .resizable(false)
        .build();
    match built {
        Ok(window) => {
            // The window may close while a new hotkey is being recorded and the
            // hotkey is paused; make sure it is registered again.
            let handle = app.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Destroyed = event {
                    settings::register_current(&handle, &handle.state::<AppState>());
                }
            });
            let _ = window.set_focus();
        }
        Err(e) => log::error!("could not open settings: {e}"),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, event| {
                    let Some(state) = app.try_state::<AppState>() else {
                        return;
                    };
                    if sc != &*state.shortcut.lock().unwrap() {
                        return;
                    }
                    match event.state() {
                        ShortcutState::Pressed => on_key_down(app),
                        ShortcutState::Released => on_key_up(app),
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::get_defaults,
            settings::get_usage,
            settings::list_input_devices,
            settings::list_local_models,
            settings::set_engine,
            settings::set_local_model,
            settings::set_input_device,
            settings::set_cleanup_model,
            settings::set_hotkey,
            settings::pause_hotkey,
            settings::resume_hotkey,
        ])
        .setup(|app| {
            let log_dir = app.path().app_log_dir()?;
            logger::init(log_dir.clone());
            // Route whisper.cpp / GGML output through our logger.
            whisper_rs::install_logging_hooks();
            log::info!("starting Dictation AI {}", env!("CARGO_PKG_VERSION"));

            // Menu bar only: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let mut loaded = Settings::load(&settings_path);
            // A hand-edited or stale hotkey must not stop the app starting.
            let shortcut: Shortcut = loaded.hotkey.parse().unwrap_or_else(|_| {
                log::warn!("saved hotkey {:?} is invalid, using default", loaded.hotkey);
                loaded.hotkey = settings::DEFAULT_HOTKEY.to_string();
                settings::DEFAULT_HOTKEY.parse().expect("default hotkey is valid")
            });
            let hotkey_label = loaded.hotkey.clone();
            let preload_model = (loaded.engine == Engine::Local).then(|| loaded.local_model.clone());
            let usage = UsageStore::load(app.path().app_data_dir()?.join("usage.json"));
            app.manage(AppState::new(loaded, shortcut.clone(), settings_path, usage));

            let settings_item =
                MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let logs_item =
                MenuItem::with_id(app, "logs", "Show Logs in Finder", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Dictation AI", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_item, &logs_item, &quit])?;

            TrayIconBuilder::with_id(status::TRAY_ID)
                .icon(status::initial_icon())
                .icon_as_template(true)
                .menu(&menu)
                .tooltip("Dictation AI")
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "settings" => open_settings(app),
                    "logs" => {
                        if let Err(e) = std::process::Command::new("open").arg(&log_dir).spawn() {
                            log::error!("could not open log folder: {e}");
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Surface the Accessibility prompt at launch rather than on first paste.
            if !paste::accessibility_trusted(true) {
                log::warn!("Accessibility permission not granted yet (needed to paste)");
            }

            if let Some(model) = preload_model {
                local::preload(app.handle(), &model);
            }

            app.global_shortcut().register(shortcut)?;
            log::info!("ready; hold {hotkey_label}, speak, release");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app, event| {
        // The settings window comes and goes; keep running as a menu bar app.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
