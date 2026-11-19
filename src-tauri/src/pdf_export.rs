//! Native "Download → PDF" for documents.
//!
//! The web app prints documents through a hidden iframe and the browser's "Save as PDF", but
//! `print()` does nothing in the desktop webview. Instead the web app hands the document's HTML
//! to `export_pdf`, which asks where to save it and renders it with the OS print engine: real,
//! selectable text and page breaks driven by the document's print CSS, like a browser's PDF.
//!
//! macOS only for now. Elsewhere the bridge does not expose `exportPdf`, so the web app keeps its
//! image-page PDF fallback.

// Rendering is macOS-only; elsewhere these helpers wait for a backend.
#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use serde::Serialize;
use tauri::{Manager, State, Webview};

use crate::downloads::{self, Downloads};

/// Generous on purpose: a long document with images can take a while to lay out and paginate.
#[cfg(target_os = "macos")]
const RENDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Upper bound on the HTML accepted over IPC. Assumption: artifact documents, images inlined as
/// data URLs included, stay well under this; it only stops a runaway payload.
const MAX_HTML_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PdfExportStatus {
    Saved,
    Cancelled,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfExportResult {
    status: PdfExportStatus,
}

impl PdfExportResult {
    fn of(status: PdfExportStatus) -> Self {
        Self { status }
    }
}

#[tauri::command]
pub async fn export_pdf(
    webview: Webview,
    downloads: State<'_, Downloads>,
    html: String,
    title: Option<String>,
    file_name: String,
) -> Result<PdfExportResult, String> {
    if webview.label() != crate::main_window::LABEL {
        return Err("PDF export requires the main webview".into());
    }
    if html.len() > MAX_HTML_BYTES {
        return Err("The document is too large to export".into());
    }
    let file_name = pdf_file_name(&file_name);

    #[cfg(target_os = "macos")]
    {
        let Some(path) = pick_destination(&webview, &file_name).await? else {
            return Ok(PdfExportResult::of(PdfExportStatus::Cancelled));
        };
        let title = title.unwrap_or_else(|| file_name.trim_end_matches(".pdf").to_string());
        match render(&webview, html, title, path.clone()).await {
            Ok(()) => {
                let id = downloads.record(path.clone());
                downloads::show_feedback(
                    &webview,
                    id.as_deref().map(|id| (id, path.as_path())),
                    true,
                );
                Ok(PdfExportResult::of(PdfExportStatus::Saved))
            }
            Err(error) => {
                log::warn!("PDF export failed: {error}");
                // Don't leave a half-written file where the user asked us to save.
                let _ = std::fs::remove_file(&path);
                Ok(PdfExportResult::of(PdfExportStatus::Failed))
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (downloads, html, title, file_name);
        Err("PDF export is not supported on this platform yet".into())
    }
}

/// The hosted artifact view loads nothing remote, so neither may the export of the same document.
const EXPORT_CSP: &str =
    "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:";

/// `html` with [`EXPORT_CSP`] as the first element of its head.
///
/// A CSP meta tag is ignored in the body and a tag before the doctype switches to quirks mode,
/// so it goes inside an explicit `<head>` that precedes `<body>`, else after the doctype.
fn with_csp(html: &str) -> String {
    let meta = format!(r#"<meta http-equiv="Content-Security-Policy" content="{EXPORT_CSP}">"#);
    let at = head_content_start(html)
        .or_else(|| doctype_end(html))
        .unwrap_or(0);
    format!("{}{meta}{}", &html[..at], &html[at..])
}

fn head_content_start(html: &str) -> Option<usize> {
    // ASCII lowercasing keeps byte offsets, so they index `html` too.
    let lower = html.to_ascii_lowercase();
    let body = lower.find("<body").unwrap_or(lower.len());
    let mut from = 0;
    while let Some(found) = lower[from..body].find("<head") {
        let after = from + found + "<head".len();
        match lower.as_bytes().get(after) {
            Some(b'>') => return Some(after + 1),
            Some(c) if c.is_ascii_whitespace() => {
                return lower[after..].find('>').map(|end| after + end + 1);
            }
            _ => from = after,
        }
    }
    None
}

fn doctype_end(html: &str) -> Option<usize> {
    let trimmed = html.trim_start();
    let offset = html.len() - trimmed.len();
    if !trimmed.to_ascii_lowercase().starts_with("<!doctype") {
        return None;
    }
    trimmed.find('>').map(|end| offset + end + 1)
}

/// A bare file name ending in `.pdf`: whatever the web app sends, never a path.
fn pdf_file_name(requested: &str) -> String {
    let name: String = requested
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim().trim_start_matches('.').trim();
    let name = if name.is_empty() { "Document" } else { name };
    if name.to_ascii_lowercase().ends_with(".pdf") {
        name.to_string()
    } else {
        format!("{name}.pdf")
    }
}

#[cfg(target_os = "macos")]
async fn pick_destination(
    webview: &Webview,
    file_name: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;

    let window = webview.window();
    let dialog = webview
        .app_handle()
        .dialog()
        .file()
        .set_parent(&window)
        .set_file_name(file_name)
        .add_filter("PDF", &["pdf"]);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|_| "Could not open the save dialog".to_string())?;
    picked
        .map(|path| {
            path.into_path()
                .map_err(|_| "Unsupported save location".to_string())
        })
        .transpose()
}

#[cfg(target_os = "macos")]
async fn render(
    webview: &Webview,
    html: String,
    title: String,
    path: std::path::PathBuf,
) -> Result<(), String> {
    let ns_window = webview.window().ns_window().map_err(|e| e.to_string())? as usize;
    let (done, result) = tokio::sync::oneshot::channel();
    webview
        .run_on_main_thread(move || macos::print_to_pdf(html, title, path, ns_window, done))
        .map_err(|e| e.to_string())?;
    match tokio::time::timeout(RENDER_TIMEOUT, result).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) => Err("PDF renderer stopped without a result".into()),
        Err(_) => Err("PDF rendering timed out".into()),
    }
}

#[cfg(target_os = "macos")]
mod macos {
    #![allow(unexpected_cfgs)]

    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use cocoa::base::{id, nil, BOOL, NO, YES};
    use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    type Done = tokio::sync::oneshot::Sender<Result<(), String>>;

    // Linked rather than spelled out: the constants' names are not their values
    // (`NSPrintJobSavingURL` is "NSJobSavingURL"), and a wrong key makes AppKit ask for a
    // destination with a second Save panel.
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {
        static NSPrintJobSavingURL: id;
        static NSPrintSaveJob: id;
    }

    /// Page margins in points, matching the documents' own `@page { margin: 18mm 16mm }`.
    /// Assumption to verify: if WebKit also applies `@page` margins when printing, these add to
    /// them and the margins come out wider; zero would instead put text against the paper edge
    /// whenever WebKit ignores `@page`, which is the worse failure.
    const MARGIN_VERTICAL: f64 = 18.0 / 25.4 * 72.0;
    const MARGIN_HORIZONTAL: f64 = 16.0 / 25.4 * 72.0;

    struct Job {
        path: PathBuf,
        title: String,
        window: usize,
        webview: id,
        navigated: bool,
        started: bool,
        done: Option<Done>,
    }

    /// Blocks every network and file load, whatever the document's markup; `data:` and
    /// `about:` stay allowed. Complements the CSP in case the meta tag lands somewhere ignored.
    const BLOCK_NETWORK_RULES: &str = r#"[
        {"trigger": {"url-filter": "^https?:"}, "action": {"type": "block"}},
        {"trigger": {"url-filter": "^wss?:"}, "action": {"type": "block"}},
        {"trigger": {"url-filter": "^ftp:"}, "action": {"type": "block"}},
        {"trigger": {"url-filter": "^file:"}, "action": {"type": "block"}}
    ]"#;

    const NAVIGATION_CANCEL: isize = 0;
    const NAVIGATION_ALLOW: isize = 1;

    fn ns_string(value: &str) -> id {
        unsafe { NSString::alloc(nil).init_str(value).autorelease() }
    }

    unsafe fn job<'a>(delegate: &Object) -> Option<&'a mut Job> {
        let ptr = *delegate.get_ivar::<usize>("job") as *mut Job;
        ptr.as_mut()
    }

    /// Reports the outcome once and tears down the hidden webview and its delegate.
    unsafe fn finish(delegate: &mut Object, outcome: Result<(), String>) {
        let ptr = *delegate.get_ivar::<usize>("job") as *mut Job;
        if ptr.is_null() {
            return;
        }
        delegate.set_ivar::<usize>("job", 0);
        let mut job = Box::from_raw(ptr);
        if let Some(done) = job.done.take() {
            let _ = done.send(outcome);
        }
        let _: () = msg_send![job.webview, setNavigationDelegate: nil];
        let _: () = msg_send![job.webview, autorelease];
        let _: () = msg_send![delegate as *mut Object, autorelease];
    }

    /// Allows only the initial load of the HTML string; refresh, links and frames are cancelled.
    extern "C" fn decide_policy(
        this: &mut Object,
        _: Sel,
        _webview: id,
        _action: id,
        decision: id,
    ) {
        unsafe {
            let decision = &*(decision as *const block::Block<(isize,), ()>);
            let allow = match job(this) {
                Some(job) if !job.navigated => {
                    job.navigated = true;
                    true
                }
                _ => false,
            };
            decision.call((if allow {
                NAVIGATION_ALLOW
            } else {
                NAVIGATION_CANCEL
            },));
        }
    }

    extern "C" fn did_finish_navigation(this: &mut Object, _: Sel, _webview: id, _navigation: id) {
        unsafe {
            let Some(job) = job(this) else { return };
            // A `<meta http-equiv="refresh">` could navigate again; print the first load only.
            if job.started {
                return;
            }
            job.started = true;
            if let Err(error) = start_print(this, job) {
                finish(this, Err(error));
            }
        }
    }

    extern "C" fn did_fail_navigation(
        this: &mut Object,
        _: Sel,
        _webview: id,
        _navigation: id,
        error: id,
    ) {
        unsafe {
            let description: id = msg_send![error, localizedDescription];
            let message = if description.is_null() {
                "The document could not be loaded".to_string()
            } else {
                let bytes = NSString::UTF8String(description);
                std::ffi::CStr::from_ptr(bytes)
                    .to_string_lossy()
                    .into_owned()
            };
            finish(this, Err(message));
        }
    }

    extern "C" fn print_did_run(
        this: &mut Object,
        _: Sel,
        _operation: id,
        success: BOOL,
        _context: *mut c_void,
    ) {
        unsafe {
            let outcome = if success == YES {
                Ok(())
            } else {
                Err("The print operation did not complete".to_string())
            };
            finish(this, outcome);
        }
    }

    fn delegate_class() -> &'static Class {
        static CLASS: OnceLock<&'static Class> = OnceLock::new();
        CLASS.get_or_init(|| {
            let mut class = ClassDecl::new("LumoPdfExportDelegate", class!(NSObject))
                .expect("PDF export delegate class must be registered once");
            class.add_ivar::<usize>("job");
            unsafe {
                class.add_method(
                    sel!(webView:decidePolicyForNavigationAction:decisionHandler:),
                    decide_policy as extern "C" fn(&mut Object, Sel, id, id, id),
                );
                class.add_method(
                    sel!(webView:didFinishNavigation:),
                    did_finish_navigation as extern "C" fn(&mut Object, Sel, id, id),
                );
                class.add_method(
                    sel!(webView:didFailNavigation:withError:),
                    did_fail_navigation as extern "C" fn(&mut Object, Sel, id, id, id),
                );
                class.add_method(
                    sel!(webView:didFailProvisionalNavigation:withError:),
                    did_fail_navigation as extern "C" fn(&mut Object, Sel, id, id, id),
                );
                class.add_method(
                    sel!(printOperationDidRun:success:contextInfo:),
                    print_did_run as extern "C" fn(&mut Object, Sel, id, BOOL, *mut c_void),
                );
            }
            class.register()
        })
    }

    unsafe fn start_print(delegate: &Object, job: &mut Job) -> Result<(), String> {
        let shared: id = msg_send![class!(NSPrintInfo), sharedPrintInfo];
        let info: id = msg_send![shared, copy];
        let info: id = msg_send![info, autorelease];

        let url: id =
            msg_send![class!(NSURL), fileURLWithPath: ns_string(&job.path.to_string_lossy())];
        let settings: id = msg_send![info, dictionary];
        let _: () = msg_send![settings, setObject: url forKey: NSPrintJobSavingURL];
        let _: () = msg_send![info, setJobDisposition: NSPrintSaveJob];
        let _: () = msg_send![info, setTopMargin: MARGIN_VERTICAL];
        let _: () = msg_send![info, setBottomMargin: MARGIN_VERTICAL];
        let _: () = msg_send![info, setLeftMargin: MARGIN_HORIZONTAL];
        let _: () = msg_send![info, setRightMargin: MARGIN_HORIZONTAL];
        let _: () = msg_send![info, setHorizontallyCentered: NO];
        let _: () = msg_send![info, setVerticallyCentered: NO];

        let operation: id = msg_send![job.webview, printOperationWithPrintInfo: info];
        if operation.is_null() {
            return Err("WebKit returned no print operation".into());
        }
        let _: () = msg_send![operation, setShowsPrintPanel: NO];
        let _: () = msg_send![operation, setShowsProgressPanel: NO];
        let _: () = msg_send![operation, setJobTitle: ns_string(&job.title)];

        // WKWebView's print view renders blank pages unless it is given a frame.
        let paper: NSSize = msg_send![info, paperSize];
        let view: id = msg_send![operation, view];
        let _: () = msg_send![view, setFrame: NSRect::new(NSPoint::new(0.0, 0.0), paper)];

        // Plain `runOperation` is reported to print WKWebView content blank; the window-modal
        // variant lays it out properly. With both panels hidden nothing is shown on the window.
        let _: () = msg_send![operation,
            runOperationModalForWindow: job.window as id
            delegate: delegate as *const Object as id
            didRunSelector: sel!(printOperationDidRun:success:contextInfo:)
            contextInfo: std::ptr::null_mut::<c_void>()];
        Ok(())
    }

    /// Loads `html` into a hidden WKWebView with JavaScript and network access off and no app
    /// bridge, then prints it to `path`. Must run on the main thread; `done` receives the outcome.
    pub(super) fn print_to_pdf(
        html: String,
        title: String,
        path: PathBuf,
        window: usize,
        done: Done,
    ) {
        let html = super::with_csp(&html);
        let pending = std::cell::RefCell::new(Some((html, title, path, done)));
        let compiled = block::ConcreteBlock::new(move |rules: id, error: id| {
            let Some((html, title, path, done)) = pending.borrow_mut().take() else {
                return;
            };
            unsafe {
                let on_main: BOOL = msg_send![class!(NSThread), isMainThread];
                if rules.is_null() || !error.is_null() || on_main != YES {
                    let _ = done.send(Err("Could not prepare the PDF renderer".into()));
                    return;
                }
                render_with_rules(rules, html, title, path, window, done);
            }
        })
        .copy();
        unsafe {
            let store: id = msg_send![class!(WKContentRuleListStore), defaultStore];
            let _: () = msg_send![store,
                compileContentRuleListForIdentifier: ns_string("LumoPdfExportBlockNetwork")
                encodedContentRuleList: ns_string(BLOCK_NETWORK_RULES)
                completionHandler: &*compiled];
        }
    }

    unsafe fn render_with_rules(
        rules: id,
        html: String,
        title: String,
        path: PathBuf,
        window: usize,
        done: Done,
    ) {
        unsafe {
            let config: id = msg_send![class!(WKWebViewConfiguration), new];
            let preferences: id = msg_send![config, preferences];
            // The HTML is model-generated: sanitized upstream, but never run its scripts here.
            let _: () = msg_send![preferences, setJavaScriptEnabled: NO];
            let content: id = msg_send![config, userContentController];
            let _: () = msg_send![content, addContentRuleList: rules];

            let shared: id = msg_send![class!(NSPrintInfo), sharedPrintInfo];
            let paper: NSSize = msg_send![shared, paperSize];
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), paper);
            let webview: id = msg_send![class!(WKWebView), alloc];
            let webview: id = msg_send![webview, initWithFrame: frame configuration: config];
            let _: () = msg_send![config, release];
            if webview.is_null() {
                let _ = done.send(Err("Could not create the PDF renderer".into()));
                return;
            }

            let delegate: id = msg_send![delegate_class(), new];
            let job = Box::new(Job {
                path,
                title,
                window,
                webview,
                navigated: false,
                started: false,
                done: Some(done),
            });
            (*delegate).set_ivar::<usize>("job", Box::into_raw(job) as usize);
            // `navigationDelegate` is weak; `delegate` stays retained until `finish`.
            let _: () = msg_send![webview, setNavigationDelegate: delegate];
            let _: id = msg_send![webview, loadHTMLString: ns_string(&html) baseURL: nil];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_gets_pdf_extension() {
        assert_eq!(pdf_file_name("Quarterly report"), "Quarterly report.pdf");
        assert_eq!(pdf_file_name("Report.PDF"), "Report.PDF");
    }

    #[test]
    fn file_name_cannot_be_a_path() {
        assert_eq!(pdf_file_name("../../etc/passwd"), "-..-etc-passwd.pdf");
        assert_eq!(pdf_file_name("a\\b:c"), "a-b-c.pdf");
        assert_eq!(pdf_file_name("bad\nname"), "bad-name.pdf");
    }

    #[test]
    fn empty_file_name_falls_back() {
        assert_eq!(pdf_file_name("   "), "Document.pdf");
        assert_eq!(pdf_file_name("..."), "Document.pdf");
    }

    fn csp_at(html: &str) -> usize {
        with_csp(html)
            .find(r#"<meta http-equiv="Content-Security-Policy""#)
            .unwrap()
    }

    #[test]
    fn the_csp_is_the_first_element_of_the_head() {
        let html = r#"<!DOCTYPE html><html><HEAD lang="en"><title>t</title></head><body><img src="https://evil.example/p"></body></html>"#;
        assert_eq!(csp_at(html), html.find("<title>").unwrap());
        assert!(with_csp(html).starts_with("<!DOCTYPE html>"));
        assert!(with_csp(html).contains(EXPORT_CSP));
    }

    #[test]
    fn the_csp_never_lands_in_a_header_or_the_body() {
        let html = "<!doctype html><body><header>x</header><head></head></body>";
        assert_eq!(csp_at(html), "<!doctype html>".len());
        assert_eq!(csp_at("<div><header>x</header></div>"), 0);
    }

    #[test]
    fn status_serializes_for_the_bridge() {
        let json = serde_json::to_string(&PdfExportResult::of(PdfExportStatus::Cancelled)).unwrap();
        assert_eq!(json, r#"{"status":"cancelled"}"#);
    }
}
