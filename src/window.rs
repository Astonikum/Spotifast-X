//! Checks saved window positions before restoring them.
//!
//! eframe already places the window on-screen. The session can still refer
//! to a monitor that was unplugged, so check before moving the window there.

/// The message box shown when eframe could not create an OpenGL context on
/// Windows, and the page its Yes button opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingOpenGl {
    pub title: String,
    pub text: String,
    pub page: &'static str,
}

/// The Store page of the OpenCL, OpenGL & Vulkan Compatibility Pack.
const COMPATIBILITY_PACK_PAGE: &str = "ms-windows-store://pdp/?ProductId=9NQPSL29BFFF";
/// The download guide, which documents the OpenGL 2.0 requirement.
const DOWNLOAD_PAGE: &str = "https://spotifast.rocks/download/";

/// Says why no window opened and where the fix is.
///
/// Snapdragon graphics drivers have no OpenGL of their own. Windows provides
/// it on ARM through Microsoft's compatibility pack, a Store app that
/// translates it to Direct3D 12, so ARM points at that pack. Elsewhere the
/// usual cause is a virtual machine or remote session with only the
/// Microsoft Basic Display Adapter, which offers OpenGL 1.1.
pub fn missing_opengl_message(locale: crate::i18n::Locale, arm: bool) -> MissingOpenGl {
    use crate::i18n::gettext;
    let title = gettext(locale, "Spotifast X cannot open its window").into_owned();
    if arm {
        return MissingOpenGl {
            title,
            text: gettext(
                locale,
                "On Windows on ARM, the OpenGL that Spotifast X draws with comes from Microsoft's OpenCL, OpenGL & Vulkan Compatibility Pack. Open its Microsoft Store page to install or repair it?",
            )
            .into_owned(),
            page: COMPATIBILITY_PACK_PAGE,
        };
    }
    let text = gettext(
        locale,
        // Translators: Keep {url} exactly as written. It becomes the address
        // of the Spotifast X download page.
        "Spotifast X needs a graphics driver with OpenGL 2.0 or newer, and this PC or virtual machine does not provide one. Install the graphics driver, or turn on GPU acceleration for the virtual machine. Open {url} to read more?",
    );
    MissingOpenGl {
        title,
        text: text.replace("{url}", DOWNLOAD_PAGE),
        page: DOWNLOAD_PAGE,
    }
}

/// Shows [`missing_opengl_message`] and opens its page on Yes, so a missing
/// OpenGL does not close the app without a word.
#[cfg(windows)]
pub fn report_missing_opengl(locale: crate::i18n::Locale) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONERROR, MB_YESNO, MessageBoxW};

    let message = missing_opengl_message(locale, cfg!(target_arch = "aarch64"));
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let title = wide(&message.title);
    let text = wide(&message.text);
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call,
    // and the box has no owner window.
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONERROR,
        )
    };
    if answer == IDYES
        && let Err(error) = open::that(message.page)
    {
        log::warn!("cannot open {}: {error}", message.page);
    }
}

/// Minimize the active window without leaving egui's macOS viewport flag stale
/// when the user later restores it from the Dock.
pub fn minimize_window(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    {
        use objc2::{MainThreadMarker, sel};
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSObjectNSDelayedPerforming;

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let window = app.currentEvent().and_then(|event| event.window(mtm));
        if let Some(window) = window.or_else(|| app.keyWindow()) {
            // Let egui finish this frame before AppKit minimizes the window.
            // SAFETY: NSWindow's selector takes one optional sender argument.
            unsafe {
                window.performSelector_withObject_afterDelay(sel!(miniaturize:), None, 0.0);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    #[cfg(target_os = "macos")]
    let _ = ctx;
}

/// The main window's smallest size with no side panel open. The page and
/// its top bar fit beside the narrowest sidebar from here.
pub const MAIN_MIN_SIZE: [f32; 2] = [760.0, 520.0];

/// A remembered main window size to open at, or `None` for the default. Under
/// the minimum is not a size anyone chose, since the window cannot be dragged
/// that small, and opening at it leaves a window too small to find (#704).
pub fn usable_size(size: Option<[f32; 2]>) -> Option<[f32; 2]> {
    size.filter(|size| size[0] >= MAIN_MIN_SIZE[0] && size[1] >= MAIN_MIN_SIZE[1])
}

static FIXED_SIZE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the main window keeps one size it was given, as demo shots ask,
/// so the interface must not raise its minimum.
pub fn fixed_size() -> bool {
    FIXED_SIZE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Pins the main window to the size it is created with.
pub fn set_fixed_size(on: bool) {
    FIXED_SIZE.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_opengl_points_arm_at_the_store_pack_and_others_at_the_guide() {
        let arm = missing_opengl_message(crate::i18n::Locale::English, true);
        assert_eq!(arm.page, COMPATIBILITY_PACK_PAGE);
        assert!(arm.text.contains("Compatibility Pack"));

        let x64 = missing_opengl_message(crate::i18n::Locale::English, false);
        assert_eq!(x64.title, "Spotifast X cannot open its window");
        assert_eq!(x64.page, "https://spotifast.rocks/download/");
        assert!(x64.text.contains("OpenGL 2.0"));
        assert!(x64.text.contains("virtual machine"));
        assert!(x64.text.contains("https://spotifast.rocks/download/"));
        assert!(!x64.text.contains("{url}"));
    }

    /// A remembered size under the minimum is not one anyone chose, since the
    /// window cannot be dragged that small. It is not restored, so the window
    /// opens at its default size instead.
    #[test]
    fn a_remembered_size_below_the_minimum_is_not_restored() {
        assert_eq!(usable_size(Some([64.0, 64.0])), None);
        assert_eq!(usable_size(Some([MAIN_MIN_SIZE[0] - 1.0, 800.0])), None);
        assert_eq!(usable_size(Some([1240.0, MAIN_MIN_SIZE[1] - 1.0])), None);
        assert_eq!(usable_size(None), None);
        for size in [MAIN_MIN_SIZE, [1240.0, 800.0], [1920.0, 1129.0]] {
            assert_eq!(usable_size(Some(size)), Some(size));
        }
    }
}
