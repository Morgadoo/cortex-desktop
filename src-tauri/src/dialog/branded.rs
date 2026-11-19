//! Custom Cortex icons on macOS and Windows; standard dialogs on other platforms.

use tauri::AppHandle;

#[cfg(windows)]
mod windows;

pub(crate) enum Layout {
    Native,
    IconOnLeft,
}

pub(crate) fn show(
    app: &AppHandle,
    title: &str,
    message: String,
    buttons: Option<(&str, &str)>,
    layout: Layout,
    callback: impl FnOnce(bool) + Send + 'static,
) {
    #[cfg(target_os = "macos")]
    {
        let title = title.to_owned();
        let buttons = buttons.map(|(ok, cancel)| (ok.to_owned(), cancel.to_owned()));
        if let Err(error) = app.run_on_main_thread(move || {
            let accepted = match layout {
                Layout::Native => macos::show_native(&title, &message),
                Layout::IconOnLeft => macos::show(&title, &message, buttons.as_ref()),
            };
            callback(accepted);
        }) {
            log::error!("Could not show Cortex dialog: {error}");
        }
    }

    #[cfg(windows)]
    {
        let _ = layout; // Native Windows task dialogs place the main icon beside the text.
        windows::show(app, title, message, buttons, callback);
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

        let _ = layout;
        // The GTK backend passes the message text to a printf-style formatter, so a
        // literal `%` (e.g. from a URL in the link prompt) must be escaped.
        let message = message.replace('%', "%%");
        let mut dialog = app.dialog().message(message).title(title);
        if let Some((ok, cancel)) = buttons {
            dialog = dialog.buttons(MessageDialogButtons::OkCancelCustom(
                ok.into(),
                cancel.into(),
            ));
        }
        dialog.show(callback);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    // objc 0.2 macros use a legacy cargo-clippy cfg.
    #![allow(unexpected_cfgs)]

    use std::sync::OnceLock;

    use cocoa::appkit::{NSBackingStoreType, NSImage, NSView, NSWindow, NSWindowStyleMask};
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    extern "C" fn clicked(_this: &Object, _selector: Sel, sender: id) {
        unsafe {
            let response: isize = msg_send![sender, tag];
            let app: id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, stopModalWithCode: response];
        }
    }

    fn button_target_class() -> &'static Class {
        static CLASS: OnceLock<&'static Class> = OnceLock::new();
        CLASS.get_or_init(|| {
            let mut class = ClassDecl::new("CortexBrandedDialogTarget", class!(NSObject))
                .expect("dialog target class must be registered once");
            unsafe {
                class.add_method(sel!(clicked:), clicked as extern "C" fn(&Object, Sel, id));
            }
            class.register()
        })
    }

    unsafe fn set_frame(view: id, frame: NSRect) {
        let _: () = msg_send![view, setFrame: frame];
    }

    unsafe fn string(text: &str) -> id {
        NSString::alloc(nil).init_str(text).autorelease()
    }

    unsafe fn label(text: &str, bold: bool, width: f64) -> (id, f64) {
        let field: id = msg_send![class!(NSTextField), wrappingLabelWithString: string(text)];
        let font: id = if bold {
            msg_send![class!(NSFont), boldSystemFontOfSize: 13.0f64]
        } else {
            msg_send![class!(NSFont), systemFontOfSize: 13.0f64]
        };
        let _: () = msg_send![field, setFont: font];
        let _: () = msg_send![field, setPreferredMaxLayoutWidth: width];
        let size: NSSize = msg_send![field, fittingSize];
        (field, size.height.ceil())
    }

    unsafe fn button(title: &str, key: &str, target: id, response: isize) -> id {
        let button: id = msg_send![class!(NSButton), buttonWithTitle: string(title)
            target: target action: sel!(clicked:)];
        let _: () = msg_send![button, setBezelStyle: 1usize]; // Rounded
        let _: () = msg_send![button, setKeyEquivalent: string(key)];
        let _: () = msg_send![button, setTag: response];
        let _: () = msg_send![button, sizeToFit];
        button
    }

    /// Preserve the standard About dialog layout, replacing only its icon.
    pub(super) fn show_native(title: &str, message: &str) -> bool {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let alert: id = msg_send![class!(NSAlert), new];
            let _: () = msg_send![alert, setAlertStyle: 1usize];
            let _: () = msg_send![alert, setMessageText: string(title)];
            let _: () = msg_send![alert, setInformativeText: string(message)];
            let bytes = include_bytes!("../../icons/128x128@2x.png");
            let data: id =
                msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() length: bytes.len()];
            let icon = NSImage::alloc(nil).initWithData_(data);
            if icon != nil {
                let _: () = msg_send![alert, setIcon: icon];
                let _: () = msg_send![icon, release];
            }
            let _: id = msg_send![alert, addButtonWithTitle: string("OK")];
            let response: isize = msg_send![alert, runModal];
            let _: () = msg_send![alert, release];
            pool.drain();
            response == 1000
        }
    }

    /// A native modal panel with an explicit horizontal layout. NSAlert chooses a vertical
    /// icon layout on recent macOS versions, so it cannot guarantee the requested placement.
    /// Called only on the main thread; the modal loop continues processing app events.
    pub(super) fn show(title: &str, message: &str, buttons: Option<&(String, String)>) -> bool {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let width = 460.0;
            let padding = 24.0;
            let icon_size = 64.0;
            let text_x = padding + icon_size + 20.0;
            let text_width = width - text_x - padding;
            let (heading, heading_height) = label(title, true, text_width);
            let (body, body_height) = label(message, false, text_width);
            let row_height = icon_size.max(heading_height + 8.0 + body_height);
            let height = padding + row_height + 24.0 + 32.0 + padding;
            let panel: id = msg_send![class!(NSPanel), alloc];
            let panel = panel.initWithContentRect_styleMask_backing_defer_(
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
                NSWindowStyleMask::NSTitledWindowMask
                    | NSWindowStyleMask::NSFullSizeContentViewWindowMask,
                NSBackingStoreType::NSBackingStoreBuffered,
                NO,
            );
            let _: () = msg_send![panel, setReleasedWhenClosed: NO];
            let _: () = msg_send![panel, setTitle: string(title)];
            let _: () = msg_send![panel, setTitleVisibility: 1usize]; // Hidden
            let _: () = msg_send![panel, setTitlebarAppearsTransparent: YES];
            let _: () = msg_send![panel, setMovableByWindowBackground: YES];
            let content: id = msg_send![panel, contentView];
            let top = height - padding;
            set_frame(
                heading,
                NSRect::new(
                    NSPoint::new(text_x, top - heading_height),
                    NSSize::new(text_width, heading_height),
                ),
            );
            set_frame(
                body,
                NSRect::new(
                    NSPoint::new(text_x, top - heading_height - 8.0 - body_height),
                    NSSize::new(text_width, body_height),
                ),
            );
            content.addSubview_(heading);
            content.addSubview_(body);

            let bytes = include_bytes!("../../icons/128x128@2x.png");
            let data: id =
                msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() length: bytes.len()];
            let icon = NSImage::alloc(nil).initWithData_(data);
            let image_view: id = msg_send![class!(NSImageView), alloc];
            let image_view = image_view.initWithFrame_(NSRect::new(
                NSPoint::new(padding, top - icon_size),
                NSSize::new(icon_size, icon_size),
            ));
            let _: () = msg_send![image_view, setImage: icon];
            content.addSubview_(image_view);
            let _: () = msg_send![image_view, release];
            let _: () = msg_send![icon, release];

            let target: id = msg_send![button_target_class(), new];
            let ok = button(buttons.map_or("OK", |(ok, _)| ok), "\r", target, 1000);
            let ok_width = NSView::frame(ok).size.width.max(92.0);
            set_frame(
                ok,
                NSRect::new(
                    NSPoint::new(width - padding - ok_width, padding),
                    NSSize::new(ok_width, 32.0),
                ),
            );
            content.addSubview_(ok);
            let cell: id = msg_send![ok, cell];
            let _: () = msg_send![panel, setDefaultButtonCell: cell];
            if let Some((_, cancel)) = buttons {
                let cancel = button(cancel, "\u{1b}", target, 1001);
                let cancel_width = NSView::frame(cancel).size.width.max(92.0);
                set_frame(
                    cancel,
                    NSRect::new(
                        NSPoint::new(width - padding - ok_width - 12.0 - cancel_width, padding),
                        NSSize::new(cancel_width, 32.0),
                    ),
                );
                content.addSubview_(cancel);
            }

            let app: id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![panel, center];
            let _: () = msg_send![panel, makeKeyAndOrderFront: nil];
            let response: isize = msg_send![app, runModalForWindow: panel];
            let _: () = msg_send![panel, close];
            let _: () = msg_send![panel, release];
            let _: () = msg_send![target, release];
            pool.drain();
            response == 1000
        }
    }
}
