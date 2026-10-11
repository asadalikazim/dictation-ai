mod audio;
mod cleanup;
mod paste;
mod settings;
mod status;
mod transcribe;

use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    ActivationPolicy, AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use settings::{AppState, Settings};
use status::Status;

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
            println!("[hotkey] PRESSED - recording");
        }
        Err(e) => {
            eprintln!("[audio] could not start recording: {e}");
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
    let model = state.settings.lock().unwrap().cleanup_model.clone();
    status::set(app, Status::Processing);
    match active.stop() {
        Ok(recording) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match process(recording, model, released_at).await {
                    Outcome::Pasted | Outcome::Discarded => status::set(&app, Status::Idle),
                    Outcome::Failed => flash_error(&app),
                }
            });
        }
        Err(e) => {
            eprintln!("[audio] could not stop recording: {e}");
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

async fn process(recording: audio::Recording, model: String, released_at: Instant) -> Outcome {
    let secs = recording.duration_secs();
    println!(
        "[audio] captured {secs:.2}s, peak {:.3}, rms {:.4}",
        recording.peak(),
        recording.rms()
    );
    if secs < MIN_CLIP_SECS {
        println!("[audio] too short, discarded");
        return Outcome::Discarded;
    }
    if recording.peak() < SILENCE_PEAK {
        println!("[audio] silent, discarded");
        return Outcome::Discarded;
    }

    let wav = match recording.to_wav() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[audio] WAV encode failed: {e}");
            return Outcome::Failed;
        }
    };
    // Debug aid: keep the last clip so capture quality can be checked by ear.
    let debug_path = std::env::temp_dir().join("dictation-ai-last.wav");
    if std::fs::write(&debug_path, &wav).is_ok() {
        println!("[audio] saved {} ({} KB)", debug_path.display(), wav.len() / 1024);
    }

    let raw = match transcribe::transcribe(&wav).await {
        Ok(text) if text.is_empty() => {
            println!("[transcribe] empty result, nothing to paste");
            return Outcome::Discarded;
        }
        Ok(text) => {
            println!(
                "[transcribe] {} ms after release: {text:?}",
                released_at.elapsed().as_millis()
            );
            text
        }
        Err(e) => {
            eprintln!("[transcribe] failed: {e}");
            return Outcome::Failed;
        }
    };

    let final_text = match cleanup::clean(&raw, &model).await {
        Ok(text) => {
            println!(
                "[cleanup] {} ms after release ({model}): {text:?}",
                released_at.elapsed().as_millis()
            );
            text
        }
        Err(e) => {
            eprintln!("[cleanup] failed, pasting raw transcript: {e}");
            raw
        }
    };

    match tauri::async_runtime::spawn_blocking(move || paste::paste_text(&final_text)).await {
        Ok(Ok(())) => {
            println!(
                "[paste] done, {} ms after release",
                released_at.elapsed().as_millis()
            );
            Outcome::Pasted
        }
        Ok(Err(e)) => {
            eprintln!("[paste] failed: {e}");
            Outcome::Failed
        }
        Err(e) => {
            eprintln!("[paste] task failed: {e}");
            Outcome::Failed
        }
    }
}

fn open_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("Dictation AI Settings")
        .inner_size(480.0, 400.0)
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
        Err(e) => eprintln!("[app] could not open settings: {e}"),
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
            settings::list_input_devices,
            settings::set_input_device,
            settings::set_cleanup_model,
            settings::set_hotkey,
            settings::pause_hotkey,
            settings::resume_hotkey,
        ])
        .setup(|app| {
            // Menu bar only: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let mut loaded = Settings::load(&settings_path);
            // A hand-edited or stale hotkey must not stop the app starting.
            let shortcut: Shortcut = loaded.hotkey.parse().unwrap_or_else(|_| {
                loaded.hotkey = settings::DEFAULT_HOTKEY.to_string();
                settings::DEFAULT_HOTKEY.parse().expect("default hotkey is valid")
            });
            let hotkey_label = loaded.hotkey.clone();
            app.manage(AppState::new(loaded, shortcut.clone(), settings_path));

            let settings_item =
                MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Dictation AI", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_item, &quit])?;

            TrayIconBuilder::with_id(status::TRAY_ID)
                .icon(status::initial_icon())
                .icon_as_template(true)
                .menu(&menu)
                .tooltip("Dictation AI")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => open_settings(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Surface the Accessibility prompt at launch rather than on first paste.
            if !paste::accessibility_trusted(true) {
                println!("[app] Accessibility permission not granted yet (needed to paste)");
            }

            app.global_shortcut().register(shortcut)?;
            println!("[app] ready; hold {hotkey_label}, speak, release");
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
