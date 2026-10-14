//! Launch at login, through macOS's SMAppService. The login item is the app
//! bundle itself, so it follows the app if it moves and appears under
//! System Settings > General > Login Items, where the user can revoke it.

use objc2_service_management::{SMAppService, SMAppServiceStatus};
use serde::Serialize;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    /// Registered and approved: the app starts at login.
    Enabled,
    /// Not registered.
    Disabled,
    /// Registered, but the user must approve it in System Settings.
    NeedsApproval,
    /// Not usable here, e.g. running from `tauri dev` and not from a bundle.
    Unavailable,
}

fn from_status(status: SMAppServiceStatus) -> LoginState {
    match status {
        SMAppServiceStatus::Enabled => LoginState::Enabled,
        SMAppServiceStatus::NotRegistered => LoginState::Disabled,
        SMAppServiceStatus::RequiresApproval => LoginState::NeedsApproval,
        _ => LoginState::Unavailable,
    }
}

pub fn state() -> LoginState {
    // SAFETY: a plain query on the main-app service object.
    from_status(unsafe { SMAppService::mainAppService().status() })
}

fn set(enabled: bool) -> Result<LoginState, String> {
    // SAFETY: register and unregister are documented thread-safe calls.
    let service = unsafe { SMAppService::mainAppService() };
    let result = unsafe {
        if enabled {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|e| e.localizedDescription().to_string())?;
    Ok(from_status(unsafe { service.status() }))
}

#[tauri::command]
pub fn get_login_item() -> LoginState {
    state()
}

#[tauri::command]
pub fn set_login_item(enabled: bool) -> Result<LoginState, String> {
    let new = set(enabled).map_err(|e| format!("could not change launch at login: {e}"))?;
    log::info!("launch at login is now {new:?}");
    Ok(new)
}

#[tauri::command]
pub fn open_login_items_settings() {
    // SAFETY: opens System Settings; takes no arguments.
    unsafe { SMAppService::openSystemSettingsLoginItems() };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_status_the_system_can_report() {
        assert_eq!(from_status(SMAppServiceStatus::Enabled), LoginState::Enabled);
        assert_eq!(from_status(SMAppServiceStatus::NotRegistered), LoginState::Disabled);
        assert_eq!(from_status(SMAppServiceStatus::RequiresApproval), LoginState::NeedsApproval);
        assert_eq!(from_status(SMAppServiceStatus::NotFound), LoginState::Unavailable);
        assert_eq!(from_status(SMAppServiceStatus(99)), LoginState::Unavailable);
    }

    #[test]
    fn states_serialise_in_the_form_the_ui_expects() {
        let json = serde_json::to_string(&LoginState::NeedsApproval).unwrap();
        assert_eq!(json, "\"needs_approval\"");
    }
}
