#![allow(unexpected_cfgs)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use cocoa::appkit::{NSImage, NSView};
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSModalPanelRunLoopMode: id;
}

extern "C" fn tick(this: &Object, _selector: Sel, _timer: id) {
    unsafe {
        let done = &*(*this.get_ivar::<usize>("done") as *const AtomicBool);
        if !done.load(Ordering::Acquire) {
            return;
        }
        let window = *this.get_ivar::<usize>("window") as id;
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let modal: id = msg_send![app, modalWindow];
        if modal == window {
            let _: () = msg_send![app, stopModal];
            // Wake the modal event loop after stopping it from a timer callback.
            let event: id = msg_send![class!(NSEvent), otherEventWithType: 15u64
                location: NSPoint::new(0.0, 0.0) modifierFlags: 0u64 timestamp: 0.0f64
                windowNumber: 0isize context: nil subtype: 0i16 data1: 0isize data2: 0isize];
            let _: () = msg_send![app, postEvent: event atStart: YES];
        }
    }
}

fn timer_target() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut class = ClassDecl::new("CortexLoadingTimer", class!(NSObject))
            .expect("loading timer class must be registered once");
        class.add_ivar::<usize>("done");
        class.add_ivar::<usize>("window");
        unsafe {
            class.add_method(sel!(tick:), tick as extern "C" fn(&Object, Sel, id));
        }
        class.register()
    })
}

pub(super) fn show(title: &str, message: &str, done: &AtomicBool) {
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let alert: id = msg_send![class!(NSAlert), new];
        let _: () = msg_send![alert, setAlertStyle: 1usize];
        let _: () =
            msg_send![alert, setMessageText: NSString::alloc(nil).init_str(title).autorelease()];
        let _: () = msg_send![alert, setInformativeText: NSString::alloc(nil).init_str(message).autorelease()];
        let bytes = include_bytes!("../../../icons/128x128@2x.png");
        let data: id = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() length: bytes.len()];
        let icon = NSImage::alloc(nil).initWithData_(data);
        if icon != nil {
            let _: () = msg_send![alert, setIcon: icon];
            let _: () = msg_send![icon, release];
        }
        let button: id = msg_send![alert, addButtonWithTitle: NSString::alloc(nil).init_str("Please wait…").autorelease()];
        let _: () = msg_send![button, setEnabled: NO];
        let bar: id = msg_send![class!(NSProgressIndicator), alloc];
        let bar = bar.initWithFrame_(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(280.0, 12.0),
        ));
        let _: () = msg_send![bar, setStyle: 0usize];
        let _: () = msg_send![bar, setIndeterminate: YES];
        let _: () = msg_send![bar, startAnimation: nil];
        let _: () = msg_send![alert, setAccessoryView: bar];
        let _: () = msg_send![bar, release];
        let _: () = msg_send![alert, layout];
        let window: id = msg_send![alert, window];
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
        let _: () = msg_send![window, center];
        let target: id = msg_send![timer_target(), new];
        (*target).set_ivar("done", done as *const AtomicBool as usize);
        (*target).set_ivar("window", window as usize);
        let timer: id = msg_send![class!(NSTimer), timerWithTimeInterval: 0.05f64
            target: target selector: sel!(tick:) userInfo: nil repeats: YES];
        let run_loop: id = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![run_loop, addTimer: timer forMode: NSModalPanelRunLoopMode];
        let _: isize = msg_send![app, runModalForWindow: window];
        let _: () = msg_send![timer, invalidate];
        let _: () = msg_send![target, release];
        let _: () = msg_send![window, orderOut: nil];
        let _: () = msg_send![alert, release];
        pool.drain();
    }
}
