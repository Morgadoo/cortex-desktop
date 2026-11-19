//! Native Windows task dialogs with an embedded Cortex main icon.

use ::windows::core::PCWSTR;
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_SIZE_TO_CONTENT, TDF_USE_HICON_MAIN,
};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

use super::super::win32::{owner, wide, Icon};

pub(super) fn show(
    app: &AppHandle,
    title: &str,
    message: String,
    buttons: Option<(&str, &str)>,
    callback: impl FnOnce(bool) + Send + 'static,
) {
    let title = title.to_owned();
    let buttons = buttons.map(|(ok, cancel)| (ok.to_owned(), cancel.to_owned()));
    let owner = owner(app);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match show_native(&title, &message, buttons.as_ref(), owner) {
            Ok(accepted) => callback(accepted),
            Err(error) => {
                // Keep the update decision available even if Windows cannot create a task dialog.
                log::error!("Could not show branded Cortex dialog: {error}");
                let mut dialog = app.dialog().message(message).title(title);
                if let Some((ok, cancel)) = buttons {
                    dialog = dialog.buttons(MessageDialogButtons::OkCancelCustom(ok, cancel));
                }
                dialog.show(callback);
            }
        }
    });
}

fn show_native(
    title: &str,
    message: &str,
    buttons: Option<&(String, String)>,
    owner: Option<isize>,
) -> ::windows::core::Result<bool> {
    let title = wide(title);
    let message = wide(message);
    let ok = wide(buttons.map_or("OK", |(ok, _)| ok));
    let cancel = buttons.map(|(_, cancel)| wide(cancel));
    let mut native_buttons = vec![TASKDIALOG_BUTTON {
        nButtonID: 1, // IDOK, including custom labels such as Update or Retry.
        pszButtonText: PCWSTR(ok.as_ptr()),
    }];
    if let Some(cancel) = &cancel {
        native_buttons.push(TASKDIALOG_BUTTON {
            nButtonID: 2, // IDCANCEL also handles Escape and the window close button.
            pszButtonText: PCWSTR(cancel.as_ptr()),
        });
    }
    unsafe {
        let icon = Icon::cortex();
        let config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: HWND(owner.unwrap_or_default() as *mut _),
            dwFlags: TDF_USE_HICON_MAIN | TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT,
            pszWindowTitle: PCWSTR(title.as_ptr()),
            pszMainInstruction: PCWSTR(title.as_ptr()),
            pszContent: PCWSTR(message.as_ptr()),
            Anonymous1: TASKDIALOGCONFIG_0 {
                hMainIcon: icon.handle(),
            },
            cButtons: native_buttons.len() as u32,
            pButtons: native_buttons.as_ptr(),
            nDefaultButton: 1,
            ..Default::default()
        };
        // tauri-build embeds the Common Controls v6 manifest required by TaskDialogIndirect.
        let mut response = 0;
        let result = TaskDialogIndirect(&config, Some(&mut response), None, None);
        result?;
        Ok(response == 1)
    }
}
