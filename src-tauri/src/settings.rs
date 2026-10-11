//! User settings: a small JSON file in the app config dir, plus the Tauri
//! commands the settings window uses to read and change them.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::audio;

pub const DEFAULT_HOTKEY: &str = "Alt+Space";
pub const DEFAULT_CLEANUP_MODEL: &str = "anthropic/claude-haiku-5.5";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub hotkey: String,
    /// Input device name; `None` means the system default.
    pub input_device: Option<String>,
    pub cleanup_model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            input_device: None,
            cleanup_model: DEFAULT_CLEANUP_MODEL.to_string(),
        }
    }
}

impl Settings {
    /// A missing or unreadable file yields defaults rather than an error.
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }
}

/// Runtime state shared by the hotkey handler, the pipeline and the commands.
pub struct AppState {
    pub recording: Mutex<Option<audio::ActiveRecording>>,
    pub settings: Mutex<Settings>,
    /// The shortcut currently registered with the OS.
    pub shortcut: Mutex<Shortcut>,
    pub settings_path: PathBuf,
}

impl AppState {
    pub fn new(settings: Settings, shortcut: Shortcut, settings_path: PathBuf) -> Self {
        Self {
            recording: Mutex::new(None),
            settings: Mutex::new(settings),
            shortcut: Mutex::new(shortcut),
            settings_path,
        }
    }

    fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let mut settings = self.settings.lock().unwrap();
        change(&mut settings);
        settings.save(&self.settings_path)?;
        Ok(settings.clone())
    }
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_defaults() -> Settings {
    Settings::default()
}

#[tauri::command]
pub fn list_input_devices() -> Vec<String> {
    audio::list_input_devices()
}

#[tauri::command]
pub fn set_input_device(
    state: State<AppState>,
    device: Option<String>,
) -> Result<Settings, String> {
    state.update(|s| s.input_device = device.filter(|d| !d.is_empty()))
}

#[tauri::command]
pub fn set_cleanup_model(state: State<AppState>, model: String) -> Result<Settings, String> {
    let model = model.trim().to_string();
    let model = if model.is_empty() { DEFAULT_CLEANUP_MODEL.to_string() } else { model };
    state.update(|s| s.cleanup_model = model)
}

/// Switch the global hotkey. On failure the old shortcut stays registered.
#[tauri::command]
pub fn set_hotkey(
    app: AppHandle,
    state: State<AppState>,
    hotkey: String,
) -> Result<Settings, String> {
    let new: Shortcut = hotkey
        .parse()
        .map_err(|e| format!("not a valid shortcut: {e}"))?;
    let old = state.shortcut.lock().unwrap().clone();

    if new != old {
        let gs = app.global_shortcut();
        let _ = gs.unregister(old.clone());
        if let Err(e) = gs.register(new.clone()) {
            let _ = gs.register(old);
            return Err(format!("could not use that shortcut: {e}"));
        }
        *state.shortcut.lock().unwrap() = new;
    }
    state.update(|s| s.hotkey = hotkey)
}

/// Release the hotkey while the settings window records a new one, so
/// pressing it does not start a dictation.
#[tauri::command]
pub fn pause_hotkey(app: AppHandle, state: State<AppState>) {
    let current = state.shortcut.lock().unwrap().clone();
    let _ = app.global_shortcut().unregister(current);
}

#[tauri::command]
pub fn resume_hotkey(app: AppHandle, state: State<AppState>) {
    register_current(&app, &state);
}

pub fn register_current(app: &AppHandle, state: &AppState) {
    let current = state.shortcut.lock().unwrap().clone();
    // Already registered is fine; this is idempotent on purpose.
    let _ = app.global_shortcut().register(current);
}
