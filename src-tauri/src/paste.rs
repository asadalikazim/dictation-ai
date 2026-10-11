//! Insert text at the cursor of whatever app has focus: put it on the
//! clipboard, synthesize Cmd+V, then restore the previous clipboard text.

use std::thread::sleep;
use std::time::Duration;

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::CFString;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

/// ANSI virtual keycode for "V" (assumes a US-style layout).
const KEYCODE_V: u16 = 9;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
}

/// Whether we may post keystrokes to other apps. With `prompt`, macOS shows
/// its "grant Accessibility access" dialog if we are not yet trusted.
pub fn accessibility_trusted(prompt: bool) -> bool {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = if prompt { CFBoolean::true_value() } else { CFBoolean::false_value() };
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
}

/// Blocking; call from a blocking-capable thread.
pub fn paste_text(text: &str) -> Result<(), String> {
    if !accessibility_trusted(true) {
        return Err(
            "Accessibility permission missing: enable it in System Settings > Privacy & \
             Security > Accessibility, then try again"
                .to_string(),
        );
    }

    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let previous = clipboard.get_text().ok();

    clipboard.set_text(text).map_err(|e| e.to_string())?;
    // Let a still-held hotkey modifier settle and the pasteboard update land.
    sleep(Duration::from_millis(80));
    send_cmd_v()?;
    // Restoring too early would paste the old clipboard instead.
    sleep(Duration::from_millis(250));

    match previous {
        Some(prev) => {
            clipboard.set_text(prev).map_err(|e| e.to_string())?;
        }
        None => {
            // Previous clipboard was not plain text (image, files, empty);
            // we cannot restore it, so leave the dictated text in place.
        }
    }
    Ok(())
}

fn send_cmd_v() -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "could not create event source".to_string())?;
    for key_down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), KEYCODE_V, key_down)
            .map_err(|_| "could not create keyboard event".to_string())?;
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}
