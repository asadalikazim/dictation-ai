//! Menu bar status indicator. Icons are drawn at runtime so there are no
//! image assets to maintain.

use tauri::image::Image;
use tauri::AppHandle;

pub const TRAY_ID: &str = "main";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Idle,
    Listening,
    Processing,
    Error,
}

const SIZE: u32 = 44;

/// Draw a filled dot or ring as RGBA, with a soft 1px edge.
fn dot(rgb: [u8; 3], ring: bool) -> Image<'static> {
    let c = (SIZE as f32 - 1.0) / 2.0;
    let outer = SIZE as f32 * 0.30;
    let inner = outer - 5.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let mut a = (outer + 0.5 - d).clamp(0.0, 1.0);
            if ring {
                a = a.min((d - inner + 0.5).clamp(0.0, 1.0));
            }
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (a * 255.0) as u8]);
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}

/// Returns (icon, is_template). Template icons are tinted by macOS to match
/// the menu bar; the colored states are not.
fn icon_for(status: Status) -> (Image<'static>, bool) {
    match status {
        Status::Idle => (dot([0, 0, 0], true), true),
        Status::Listening => (dot([235, 64, 52], false), false),
        Status::Processing => (dot([245, 166, 35], false), false),
        Status::Error => (dot([235, 64, 52], true), false),
    }
}

pub fn initial_icon() -> Image<'static> {
    icon_for(Status::Idle).0
}

pub fn set(app: &AppHandle, status: Status) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let (icon, template) = icon_for(status);
    let _ = tray.set_icon(Some(icon));
    let _ = tray.set_icon_as_template(template);
    let _ = tray.set_tooltip(Some(match status {
        Status::Idle => "Dictation AI",
        Status::Listening => "Dictation AI: listening",
        Status::Processing => "Dictation AI: processing",
        Status::Error => "Dictation AI: something went wrong (see console)",
    }));
}
