mod audio;
mod cleanup;
mod paste;
mod status;
mod transcribe;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{ActivationPolicy, AppHandle, Manager, RunEvent};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

use status::Status;

/// Clips shorter than this are treated as accidental taps.
const MIN_CLIP_SECS: f32 = 0.3;
/// Clips whose loudest sample is below this are treated as silence.
const SILENCE_PEAK: f32 = 0.005;
/// How long the error icon stays up before returning to idle.
const ERROR_FLASH: Duration = Duration::from_secs(3);

/// Placeholder hotkey; becomes a setting later.
fn default_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT), Code::Space)
}

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<audio::ActiveRecording>>,
}

enum Outcome {
    Pasted,
    Discarded,
    Failed,
}

fn on_key_down(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut slot = state.recording.lock().unwrap();
    if slot.is_some() {
        return; // auto-repeat or double press
    }
    match audio::start() {
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
    status::set(app, Status::Processing);
    match active.stop() {
        Ok(recording) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match process(recording, released_at).await {
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
        sleep_off_thread(ERROR_FLASH).await;
        // Don't stomp a recording that started during the flash.
        if app.state::<AppState>().recording.lock().unwrap().is_none() {
            status::set(&app, Status::Idle);
        }
    });
}

async fn sleep_off_thread(d: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

async fn process(recording: audio::Recording, released_at: Instant) -> Outcome {
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

    let final_text = match cleanup::clean(&raw).await {
        Ok(text) => {
            println!(
                "[cleanup] {} ms after release: {text:?}",
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shortcut = default_shortcut();

    let app = tauri::Builder::default()
        .manage(AppState::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, sc, event| {
                    if sc != &shortcut {
                        return;
                    }
                    match event.state() {
                        ShortcutState::Pressed => on_key_down(app),
                        ShortcutState::Released => on_key_up(app),
                    }
                })
                .build(),
        )
        .setup(|app| {
            // Menu bar only: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            let quit = MenuItem::with_id(app, "quit", "Quit Dictation AI", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            TrayIconBuilder::with_id(status::TRAY_ID)
                .icon(status::initial_icon())
                .icon_as_template(true)
                .menu(&menu)
                .tooltip("Dictation AI")
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;

            // Surface the Accessibility prompt at launch rather than on first paste.
            if !paste::accessibility_trusted(true) {
                println!("[app] Accessibility permission not granted yet (needed to paste)");
            }

            app.global_shortcut().register(default_shortcut())?;
            println!("[app] ready; hold Option+Space, speak, release");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app, event| {
        // No windows by design: keep running as a menu bar app.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
