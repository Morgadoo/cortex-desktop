//! Microphone access for dictation, in the hosted web app or the native app's own page only.
//!
//! wry grants every media-capture request on macOS, whatever the origin or frame, and the
//! Linux hook used to allow any audio request. Model-generated script runs on other origins
//! (the artifact shell on lumo-api), so access is limited to the web app's own origin.

/// Whether a page at `url` may use the microphone: the exact `LUMO_URL` origin, and in the
/// native app its own bundled page (for dictation in the composer). Mini apps run on the
/// `lumoapp` scheme in sandboxed frames and never qualify.
pub fn is_allowed(url: &url::Url) -> bool {
    let hosted = crate::LUMO_URL
        .parse::<url::Url>()
        .is_ok_and(|lumo| lumo.origin() == url.origin());
    hosted || (!crate::HOSTED_WEB_APP && is_app_page(url))
}

/// Like `is_allowed`, also allowing the page as `tauri dev` serves it: from its own server
/// (`build.devUrl`, `http://127.0.0.1:1430` when the CLI serves the built page itself), in debug
/// builds only, and only that exact origin.
pub fn is_allowed_in(url: &url::Url, dev_url: Option<&url::Url>) -> bool {
    is_allowed(url)
        || (cfg!(debug_assertions)
            && !crate::HOSTED_WEB_APP
            && dev_url.is_some_and(|dev| dev.origin() == url.origin()))
}

/// The app's own bundled page: `tauri://localhost` on macOS and Linux, `http://tauri.localhost`
/// on Windows.
fn is_app_page(url: &url::Url) -> bool {
    let host = url.host_str()
        == Some(if url.scheme() == "tauri" {
            "localhost"
        } else {
            "tauri.localhost"
        });
    host && matches!(url.scheme(), "tauri" | "http" | "https") && url.port().is_none()
}

#[cfg(target_os = "macos")]
pub use macos::install;

#[cfg(target_os = "macos")]
mod macos {
    #![allow(unexpected_cfgs)]

    use log::warn;
    use objc::runtime::{class_getInstanceMethod, method_setImplementation, Imp, Object, Sel};
    use objc::runtime::{BOOL, NO};
    use objc::{msg_send, sel, sel_impl};

    /// Clang block ABI header, enough to call a block taking a `WKPermissionDecision`.
    #[repr(C)]
    struct DecisionBlock {
        isa: *const std::ffi::c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut DecisionBlock, isize),
    }

    const WK_MEDIA_CAPTURE_TYPE_MICROPHONE: isize = 1;
    const WK_PERMISSION_DECISION_GRANT: isize = 1;
    const WK_PERMISSION_DECISION_DENY: isize = 2;

    type RequestPermission = extern "C" fn(
        &Object,
        Sel,
        *mut Object,
        *mut Object,
        *mut Object,
        isize,
        *mut DecisionBlock,
    );

    extern "C" fn request_permission(
        _this: &Object,
        _sel: Sel,
        _webview: *mut Object,
        origin: *mut Object,
        frame: *mut Object,
        capture_type: isize,
        handler: *mut DecisionBlock,
    ) {
        let grant = capture_type == WK_MEDIA_CAPTURE_TYPE_MICROPHONE
            && is_main_frame(frame)
            && origin_url(origin).is_some_and(|url| super::is_allowed(&url));
        let decision = if grant {
            WK_PERMISSION_DECISION_GRANT
        } else {
            warn!("refused a media capture request (type {capture_type})");
            WK_PERMISSION_DECISION_DENY
        };
        unsafe { ((*handler).invoke)(handler, decision) };
    }

    fn is_main_frame(frame: *mut Object) -> bool {
        if frame.is_null() {
            return false;
        }
        let is_main: BOOL = unsafe { msg_send![frame, isMainFrame] };
        is_main != NO
    }

    fn string(value: *mut Object) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let bytes: *const std::ffi::c_char = unsafe { msg_send![value, UTF8String] };
        (!bytes.is_null()).then(|| {
            unsafe { std::ffi::CStr::from_ptr(bytes) }
                .to_string_lossy()
                .into_owned()
        })
    }

    /// `WKSecurityOrigin` as a URL; port 0 means the scheme's default.
    fn origin_url(origin: *mut Object) -> Option<url::Url> {
        if origin.is_null() {
            return None;
        }
        let (protocol, host, port): (*mut Object, *mut Object, isize) = unsafe {
            (
                msg_send![origin, protocol],
                msg_send![origin, host],
                msg_send![origin, port],
            )
        };
        let (protocol, host) = (string(protocol)?, string(host)?);
        let port = if port > 0 {
            format!(":{port}")
        } else {
            String::new()
        };
        format!("{protocol}://{host}{port}").parse().ok()
    }

    /// Replaces the method on wry's UI delegate class, so every webview the app creates is
    /// covered. Taken from the live delegate, since objc2 names the class with wry's version.
    pub fn install(window: &tauri::WebviewWindow) {
        if let Err(e) = window.with_webview(|webview| unsafe {
            let delegate: *mut Object = msg_send![webview.inner() as *mut Object, UIDelegate];
            if delegate.is_null() {
                warn!("webview has no UI delegate, microphone requests use WebKit's default");
                return;
            }
            let method = class_getInstanceMethod(
                (*delegate).class(),
                sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:),
            );
            if method.is_null() {
                warn!("UI delegate has no media capture method, WebKit's default applies");
                return;
            }
            let imp: Imp = std::mem::transmute(request_permission as RequestPermission);
            method_setImplementation(method as *mut _, imp);
        }) {
            warn!("could not hook microphone permissions: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_allowed;

    #[test]
    fn only_the_web_app_origin_may_use_the_microphone() {
        let url = |s: &str| s.parse::<url::Url>().unwrap();
        assert!(is_allowed(&url(crate::LUMO_URL)));
        assert!(is_allowed(&url(&format!("{}/u/0/chat", crate::LUMO_URL))));

        let api = crate::LUMO_URL.replace("//lumo.", "//lumo-api.");
        assert!(!is_allowed(&url(&format!("{api}/lumo/v1/artifact-shell"))));
        assert!(!is_allowed(&url("https://evil.example/")));
        // The native app's own page may dictate; nothing else on a local scheme may.
        assert_eq!(
            is_allowed(&url("tauri://localhost/index.html")),
            !crate::HOSTED_WEB_APP
        );
        assert_eq!(
            is_allowed(&url("http://tauri.localhost/index.html")),
            !crate::HOSTED_WEB_APP
        );
        assert!(!is_allowed(&url("lumoapp://localhost/app-id")));
        assert!(!is_allowed(&url("http://lumoapp.localhost/app-id")));
        assert!(!is_allowed(&url("http://tauri.localhost:8080/")));
    }

    #[test]
    fn the_dev_server_page_may_dictate_in_debug_builds_only() {
        let url = |s: &str| s.parse::<url::Url>().unwrap();
        let dev = url("http://127.0.0.1:1430");
        assert_eq!(
            super::is_allowed_in(&url("http://127.0.0.1:1430/"), Some(&dev)),
            cfg!(debug_assertions) && !crate::HOSTED_WEB_APP
        );
        // Another local server, or no dev server at all, is not the app's page.
        assert!(!super::is_allowed_in(
            &url("http://127.0.0.1:8000/"),
            Some(&dev)
        ));
        assert!(!super::is_allowed_in(&url("http://127.0.0.1:1430/"), None));
    }
}
