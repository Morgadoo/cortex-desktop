use std::sync::atomic::{AtomicBool, Ordering};

use ::windows::core::{HRESULT, PCWSTR};
use ::windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use ::windows::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON,
    TASKDIALOG_NOTIFICATIONS, TDF_CALLBACK_TIMER, TDF_SHOW_MARQUEE_PROGRESS_BAR,
    TDF_USE_HICON_MAIN, TDM_CLICK_BUTTON, TDM_ENABLE_BUTTON, TDM_SET_PROGRESS_BAR_MARQUEE,
    TDN_CREATED, TDN_TIMER,
};
use ::windows::Win32::UI::WindowsAndMessaging::SendMessageW;

use super::super::win32::{wide, Icon};

unsafe extern "system" fn callback(
    window: HWND,
    notification: TASKDIALOG_NOTIFICATIONS,
    _wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> HRESULT {
    let done = unsafe { &*(data as *const AtomicBool) };
    unsafe {
        if notification == TDN_CREATED {
            SendMessageW(
                window,
                TDM_ENABLE_BUTTON.0 as u32,
                Some(WPARAM(1)),
                Some(LPARAM(0)),
            );
            SendMessageW(
                window,
                TDM_SET_PROGRESS_BAR_MARQUEE.0 as u32,
                Some(WPARAM(1)),
                Some(LPARAM(30)),
            );
        }
        if (notification == TDN_CREATED || notification == TDN_TIMER)
            && done.load(Ordering::Acquire)
        {
            SendMessageW(
                window,
                TDM_ENABLE_BUTTON.0 as u32,
                Some(WPARAM(1)),
                Some(LPARAM(1)),
            );
            SendMessageW(window, TDM_CLICK_BUTTON.0 as u32, Some(WPARAM(1)), None);
        }
    }
    HRESULT(0)
}

pub(super) fn show(
    title: &str,
    message: &str,
    done: &AtomicBool,
    owner: Option<isize>,
) -> ::windows::core::Result<()> {
    let title = wide(title);
    let message = wide(message);
    let wait = wide("Please wait…");
    let button = TASKDIALOG_BUTTON {
        nButtonID: 1,
        pszButtonText: PCWSTR(wait.as_ptr()),
    };
    unsafe {
        let icon = Icon::cortex();
        let config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: HWND(owner.unwrap_or_default() as *mut _),
            dwFlags: TDF_CALLBACK_TIMER | TDF_SHOW_MARQUEE_PROGRESS_BAR | TDF_USE_HICON_MAIN,
            pszWindowTitle: PCWSTR(title.as_ptr()),
            pszMainInstruction: PCWSTR(title.as_ptr()),
            pszContent: PCWSTR(message.as_ptr()),
            Anonymous1: TASKDIALOGCONFIG_0 {
                hMainIcon: icon.handle(),
            },
            cButtons: 1,
            pButtons: &button,
            pfCallback: Some(callback),
            // TaskDialogIndirect returns before this borrowed completion flag is released.
            lpCallbackData: done as *const AtomicBool as isize,
            ..Default::default()
        };
        TaskDialogIndirect(&config, None, None, None)
    }
}
