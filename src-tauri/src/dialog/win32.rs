use ::windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, DestroyIcon, HICON, LR_DEFAULTCOLOR,
};
use tauri::{AppHandle, Manager};

pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn owner(app: &AppHandle) -> Option<isize> {
    app.get_webview_window(crate::main_window::LABEL)
        .filter(|window| {
            window.is_visible().unwrap_or(false) && window.is_enabled().unwrap_or(false)
        })
        .and_then(|window| window.hwnd().ok())
        .map(|hwnd| hwnd.0 as isize)
}

pub(super) struct Icon(Option<HICON>);

impl Icon {
    pub(super) fn cortex() -> Self {
        let icon = unsafe {
            CreateIconFromResourceEx(
                include_bytes!("../../icons/128x128@2x.png"),
                true,
                0x00030000,
                64,
                64,
                LR_DEFAULTCOLOR,
            )
        };
        match icon {
            Ok(icon) => Self(Some(icon)),
            Err(error) => {
                log::warn!("Could not load Cortex dialog icon: {error}");
                Self(None)
            }
        }
    }

    pub(super) fn handle(&self) -> HICON {
        self.0.unwrap_or_default()
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        if let Some(icon) = self.0 {
            let _ = unsafe { DestroyIcon(icon) };
        }
    }
}
