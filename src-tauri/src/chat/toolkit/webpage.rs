//! A real browser for the assistant (Onyx's sandboxed browser, Osaurus's browser session per
//! agent, OpenBot's browser profiles): the `browser_view` tool opens a page in the user's own
//! Chrome, Chromium, Edge or Brave, headless, after its scripts run, and returns a screenshot
//! and/or the rendered text. Each agent has a browser profile of its own (cookies, logins), kept
//! in `chat/browser-profiles/<agent>`; the user signs an agent in to sites by opening that
//! profile in a visible window from the agent's page.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};

use cortex_kernel::tool::{ChatTool, Outcome};

pub const BROWSER_VIEW: &str = "browser_view";
const TIMEOUT_SECS: u64 = 45;
/// Characters of a page's text returned to the model.
const MAX_TEXT: usize = 20_000;

/// The browser to drive: `CORTEX_BROWSER`, else the first Chromium-family browser found.
pub(in crate::chat::toolkit) fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CORTEX_BROWSER")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in [
            "Google Chrome",
            "Chromium",
            "Microsoft Edge",
            "Brave Browser",
        ] {
            candidates.push(PathBuf::from(format!(
                "/Applications/{app}.app/Contents/MacOS/{app}"
            )));
        }
    } else if cfg!(windows) {
        for base in [
            std::env::var("ProgramFiles").unwrap_or_default(),
            std::env::var("ProgramFiles(x86)").unwrap_or_default(),
            std::env::var("LocalAppData").unwrap_or_default(),
        ] {
            if base.is_empty() {
                continue;
            }
            for rel in [
                r"Google\Chrome\Application\chrome.exe",
                r"Microsoft\Edge\Application\msedge.exe",
                r"BraveSoftware\Brave-Browser\Application\brave.exe",
                r"Chromium\Application\chrome.exe",
            ] {
                candidates.push(Path::new(&base).join(rel));
            }
        }
    } else {
        for name in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "microsoft-edge-stable",
            "brave-browser",
        ] {
            for dir in std::env::var("PATH").unwrap_or_default().split(':') {
                candidates.push(Path::new(dir).join(name));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The profile folder of an agent (or of the assistant when `agent` is `None`).
pub(in crate::chat::toolkit) fn profile_dir(config_dir: &Path, agent: Option<&str>) -> PathBuf {
    let name: String = agent
        .unwrap_or("assistant")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();
    crate::chat::store::chat_dir(config_dir)
        .join("browser-profiles")
        .join(if name.is_empty() {
            "assistant".to_string()
        } else {
            name
        })
}

/// The arguments for one headless run.
pub(in crate::chat::toolkit) fn headless_args(
    profile: &Path,
    url: &str,
    width: u32,
    height: u32,
    screenshot: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "--headless=new".to_string(),
        "--disable-gpu".into(),
        "--hide-scrollbars".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--mute-audio".into(),
        // A new profile must not wait on the keyring or on Google's services to start.
        "--password-store=basic".into(),
        "--use-mock-keychain".into(),
        "--disable-background-networking".into(),
        "--disable-component-update".into(),
        "--disable-domain-reliability".into(),
        "--disable-sync".into(),
        "--disable-default-apps".into(),
        "--disable-extensions".into(),
        "--disable-client-side-phishing-detection".into(),
        "--disable-field-trial-config".into(),
        "--metrics-recording-only".into(),
        "--no-pings".into(),
        "--disable-features=OptimizationGuideModelDownloading,OptimizationHintsFetching,MediaRouter,Translate,InterestFeedContentSuggestions".into(),
        format!("--user-data-dir={}", profile.display()),
        format!("--window-size={width},{height}"),
        // Gives scripts a moment after load, so pages that render late still show.
        "--virtual-time-budget=5000".into(),
    ];
    // Root (containers) cannot use Chromium's sandbox.
    #[cfg(unix)]
    if unsafe { libc_geteuid() } == 0 {
        args.push("--no-sandbox".into());
    }
    match screenshot {
        Some(path) => args.push(format!("--screenshot={}", path.display())),
        None => args.push("--dump-dom".into()),
    }
    args.push(url.to_string());
    args
}

#[cfg(unix)]
unsafe fn libc_geteuid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

async fn run(browser: &Path, args: Vec<String>) -> Result<Vec<u8>, String> {
    let mut cmd = tokio::process::Command::new(browser);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| format!("The browser could not start: {e}"))?;
    match tokio::time::timeout(
        std::time::Duration::from_secs(TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(out)) => Ok(out.stdout),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("The page took more than {TIMEOUT_SECS} seconds.")),
    }
}

/// The page's title and readable text from its rendered HTML.
pub(in crate::chat::toolkit) fn page_text(html: &str) -> (String, String) {
    let doc = scraper::Html::parse_document(html);
    let title = scraper::Selector::parse("title")
        .ok()
        .and_then(|s| doc.select(&s).next())
        .map(|t| t.text().collect::<String>().trim().to_string())
        .unwrap_or_default();
    let body = scraper::Selector::parse("body")
        .ok()
        .and_then(|s| doc.select(&s).next());
    let mut text = String::new();
    if let Some(body) = body {
        for node in body.descendants() {
            if let Some(el) = node.value().as_element() {
                if matches!(
                    el.name(),
                    "p" | "div"
                        | "li"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "tr"
                        | "br"
                        | "section"
                        | "article"
                ) && !text.ends_with('\n')
                {
                    text.push('\n');
                }
            } else if let Some(t) = node.value().as_text() {
                let skip = node.ancestors().any(|a| {
                    a.value().as_element().is_some_and(|e| {
                        matches!(
                            e.name(),
                            "script" | "style" | "noscript" | "svg" | "template"
                        )
                    })
                });
                if !skip {
                    let raw: &str = t;
                    let words = raw.split_whitespace().collect::<Vec<_>>().join(" ");
                    // Space between pieces only where the page had some.
                    if raw.starts_with(char::is_whitespace)
                        && !text.is_empty()
                        && !text.ends_with(['\n', ' '])
                    {
                        text.push(' ');
                    }
                    text.push_str(&words);
                    if raw.ends_with(char::is_whitespace) && !words.is_empty() {
                        text.push(' ');
                    }
                }
            }
        }
    }
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect();
    (title, lines.join("\n"))
}

pub fn definition() -> Option<ChatTool> {
    find_browser()?;
    Some(ChatTool {
        name: BROWSER_VIEW.into(),
        description: "Open a web page in a real browser (scripts run, the user's or this agent's saved logins apply) and \
                      get a screenshot, the rendered text, or both. Use it for pages that need JavaScript or a login, or \
                      when the user wants to see the page; use browser__fetch for plain pages. The screenshot is shown \
                      to the user and saved in this chat's workspace."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "capture": { "type": "string", "enum": ["screenshot", "text", "both"], "default": "both" },
                "full_page": { "type": "boolean", "description": "A tall screenshot of more of the page." },
                "mobile": { "type": "boolean", "description": "A phone-sized window." }
            },
            "required": ["url"]
        }),
        connector_id: None,
        tool_name: BROWSER_VIEW.into(),
        read_only: true,
    })
}

/// Runs `browser_view` for a chat (and its agent's profile).
pub async fn view(
    config_dir: &Path,
    conversation_id: &str,
    agent: Option<&str>,
    args: &Value,
) -> Outcome {
    let err = |text: String| Outcome {
        text,
        is_error: true,
    };
    let Some(browser) = find_browser() else {
        return err("No Chrome, Chromium, Edge or Brave is installed.".into());
    };
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return err("Give a full http(s) address.".into());
    }
    let capture = args
        .get("capture")
        .and_then(Value::as_str)
        .unwrap_or("both");
    let (width, height) = if args.get("mobile").and_then(Value::as_bool).unwrap_or(false) {
        (412, 915)
    } else {
        (1280, 900)
    };
    let height = if args
        .get("full_page")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        height * 4
    } else {
        height
    };
    let profile = profile_dir(config_dir, agent);
    if let Err(e) = std::fs::create_dir_all(&profile) {
        return err(e.to_string());
    }
    let mut out = json!({ "url": url });
    if capture != "text" {
        let dir = match crate::chat::sandbox::workspace(config_dir, conversation_id) {
            Ok(d) => d.join("images"),
            Err(e) => return err(e),
        };
        let _ = std::fs::create_dir_all(&dir);
        let name = format!(
            "page-{}.png",
            chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
        );
        let path = dir.join(&name);
        if let Err(e) = run(
            &browser,
            headless_args(&profile, &url, width, height, Some(&path)),
        )
        .await
        {
            return err(e);
        }
        if !path.is_file() {
            return err(
                "The browser made no screenshot (the page may have refused to load).".into(),
            );
        }
        out["file"] = json!(format!("images/{name}"));
    }
    if capture != "screenshot" {
        match run(&browser, headless_args(&profile, &url, width, height, None)).await {
            Ok(html) => {
                let (title, text) = page_text(&String::from_utf8_lossy(&html));
                out["title"] = json!(title);
                out["text"] = json!(text.chars().take(MAX_TEXT).collect::<String>());
            }
            Err(e) => return err(e),
        }
    }
    Outcome::ok(out.to_string())
}

/// Opens an agent's profile in a visible browser window, to sign in to sites for it.
pub fn open_profile(config_dir: &Path, agent: Option<&str>, url: &str) -> Result<(), String> {
    let browser = find_browser().ok_or("No Chrome, Chromium, Edge or Brave is installed.")?;
    let profile = profile_dir(config_dir, agent);
    std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(browser);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check");
    #[cfg(unix)]
    if unsafe { libc_geteuid() } == 0 {
        cmd.arg("--no-sandbox");
    }
    cmd.arg(if url.starts_with("http") {
        url
    } else {
        "about:blank"
    });
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Forgets an agent's browser session (its cookies and logins).
pub fn clear_profile(config_dir: &Path, agent: Option<&str>) -> Result<(), String> {
    match std::fs::remove_dir_all(profile_dir(config_dir, agent)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_read_as_text_and_profiles_stay_apart() {
        let (title, text) = page_text("<html><head><title> Shop </title><style>.x{}</style></head><body><h1>Prices</h1><script>var a=1</script><p>Pi 5 <b>4GB</b>: 64 EUR</p><ul><li>One</li><li>Two</li></ul></body></html>");
        assert_eq!(title, "Shop");
        assert_eq!(text, "Prices\nPi 5 4GB: 64 EUR\nOne\nTwo");
        let dir = Path::new("/cfg");
        assert_ne!(profile_dir(dir, Some("a1")), profile_dir(dir, Some("a2")));
        assert!(
            profile_dir(dir, Some("../../etc")).ends_with("etc"),
            "no path tricks"
        );
        assert!(profile_dir(dir, None).ends_with("assistant"));
        let args = headless_args(
            Path::new("/p"),
            "https://x.test",
            1280,
            900,
            Some(Path::new("/o.png")),
        );
        assert!(
            args.contains(&"--screenshot=/o.png".to_string())
                && args.contains(&"--user-data-dir=/p".to_string())
                && args.last().unwrap() == "https://x.test"
        );
        assert!(headless_args(Path::new("/p"), "https://x.test", 1, 1, None)
            .contains(&"--dump-dom".to_string()));
    }

    /// Runs only where a browser is installed.
    #[tokio::test]
    async fn a_real_browser_screenshots_and_reads_a_page() {
        if find_browser().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("p.html");
        std::fs::write(&page, "<title>T</title><body><p id=x>static</p><script>document.getElementById('x').textContent='made by script'</script></body>").unwrap();
        // A file URL stands in for a site: the same browser path runs.
        let browser = find_browser().unwrap();
        let profile = profile_dir(dir.path(), Some("a1"));
        std::fs::create_dir_all(&profile).unwrap();
        let url = format!("file://{}", page.display());
        let html = run(&browser, headless_args(&profile, &url, 800, 600, None))
            .await
            .unwrap();
        let (_, text) = page_text(&String::from_utf8_lossy(&html));
        assert_eq!(text, "made by script", "scripts ran");
        let shot = dir.path().join("s.png");
        run(
            &browser,
            headless_args(&profile, &url, 800, 600, Some(&shot)),
        )
        .await
        .unwrap();
        assert!(std::fs::read(&shot).unwrap().starts_with(b"\x89PNG"));
    }
}

// ---------------------------------------------------------------------------------------------
// Driving a page (OpenBot's browser control): `browser_act` opens a page with the agent's
// profile and runs a list of steps (click, type, press a key, wait, go to another page) through
// the browser's DevTools protocol, then returns a screenshot and the page's text.

pub const BROWSER_ACT: &str = "browser_act";
const MAX_STEPS: usize = 20;

pub fn act_definition() -> Option<ChatTool> {
    find_browser()?;
    Some(ChatTool {
        name: BROWSER_ACT.into(),
        description: "Use a web page like a person: open `url` in a real browser (with this agent's logins) and run \
                      `steps` in order, then get a screenshot and the page's text. Steps: {\"click\": target}, \
                      {\"type\": text, \"into\": target}, {\"press\": \"Enter\"|\"Tab\"|\"Escape\"}, {\"wait\": ms}, \
                      {\"goto\": url}. A target is a CSS selector, or `text=Sign in` for the element whose text, placeholder or label \
                      says that. Every result lists the page's `controls` with targets that find them; with an unknown page, \
                      start with no steps to see them. \
                      Do not buy, send, delete or publish anything unless the user asked for exactly that."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "steps": { "type": "array", "maxItems": MAX_STEPS, "items": { "type": "object" } }
            },
            "required": ["url", "steps"]
        }),
        connector_id: None,
        tool_name: BROWSER_ACT.into(),
        read_only: false,
    })
}

/// JavaScript that finds a target (CSS selector or `text=…`) and returns it, or null.
fn find_js(target: &str) -> String {
    let t = serde_json::to_string(target).unwrap_or_default();
    format!(
        "(() => {{ const t = {t}; if (t.startsWith('text=')) {{ const want = t.slice(5).trim().toLowerCase(); \
         const all = [...document.querySelectorAll('input,textarea,select,a,button,[role=button],label,summary,li,span,div')]; \
         const name = e => [e.innerText, e.value, e.placeholder, e.getAttribute('aria-label'), e.title].filter(Boolean).join(' ').trim().toLowerCase(); \
         return all.find(e => name(e) === want) || all.find(e => name(e).includes(want)) || null; }} \
         try {{ return document.querySelector(t); }} catch (e) {{ return null; }} }})()"
    )
}

/// What can be used on the page, each with a target that finds it: what the model picks from.
const CONTROLS_JS: &str = "JSON.stringify([...document.querySelectorAll('input:not([type=hidden]),textarea,select,button,a[href],[role=button]')].filter(e => e.offsetParent !== null).slice(0, 40).map(e => { \
  const label = (e.innerText || e.value || e.placeholder || e.getAttribute('aria-label') || e.name || '').trim().replace(/\\s+/g, ' ').slice(0, 60); \
  const target = e.id ? '#' + e.id : e.name ? e.tagName.toLowerCase() + '[name=\"' + e.name + '\"]' : label ? 'text=' + label : e.tagName.toLowerCase(); \
  return e.tagName.toLowerCase() + (e.type && e.tagName === 'INPUT' ? '[' + e.type + ']' : '') + ' \"' + label + '\" → ' + target; }))";

/// The script for one step; it returns a short report, or throws when the target is missing.
pub(in crate::chat::toolkit) fn step_js(step: &Value) -> Result<String, String> {
    let s = |k: &str| step.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(target) = s("click") {
        return Ok(format!("(() => {{ const el = {}; if (!el) throw new Error('nothing matches {}'); el.scrollIntoView({{block:'center'}}); el.click(); return 'clicked'; }})()", find_js(&target), target.replace('\'', "")));
    }
    if let Some(text) = s("type") {
        let target = s("into").unwrap_or_else(|| ":focus".into());
        let value = serde_json::to_string(&text).unwrap_or_default();
        return Ok(format!(
            "(() => {{ const el = {}; if (!el) throw new Error('nothing matches {}'); el.focus(); \
             const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; \
             const set = Object.getOwnPropertyDescriptor(proto, 'value'); if (set && 'value' in el) set.set.call(el, {value}); else el.textContent = {value}; \
             el.dispatchEvent(new Event('input', {{bubbles:true}})); el.dispatchEvent(new Event('change', {{bubbles:true}})); return 'typed'; }})()",
            find_js(&target),
            target.replace('\'', "")
        ));
    }
    if let Some(key) = s("press") {
        if !matches!(key.as_str(), "Enter" | "Tab" | "Escape") {
            return Err(format!(
                "Only Enter, Tab and Escape can be pressed, not {key}."
            ));
        }
        // Enter on a field submits its form, as a person's would.
        return Ok(format!(
            "(() => {{ const el = document.activeElement || document.body; const k = '{key}'; \
             for (const type of ['keydown','keypress','keyup']) el.dispatchEvent(new KeyboardEvent(type, {{key:k, code:k, bubbles:true}})); \
             if (k === 'Enter' && el.form) {{ el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit(); }} return 'pressed ' + k; }})()"
        ));
    }
    Err(format!("Unknown step {step}."))
}

struct Cdp {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next: u64,
    session: Option<String>,
}

impl Cdp {
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        use futures::{SinkExt as _, StreamExt as _};
        self.next += 1;
        let id = self.next;
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = &self.session {
            msg["sessionId"] = json!(s);
        }
        self.ws
            .send(tokio_tungstenite::tungstenite::Message::Text(
                msg.to_string().into(),
            ))
            .await
            .map_err(|e| e.to_string())?;
        loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(30), self.ws.next())
                .await
                .map_err(|_| format!("The browser did not answer {method}."))?;
            let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = frame else {
                if frame.is_none() {
                    return Err("The browser closed.".into());
                }
                continue;
            };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["id"].as_u64() == Some(id) {
                if let Some(e) = v.get("error") {
                    return Err(e["message"].as_str().unwrap_or("browser error").to_string());
                }
                return Ok(v["result"].clone());
            }
        }
    }

    async fn eval(&mut self, script: &str) -> Result<Value, String> {
        let r = self.call("Runtime.evaluate", json!({ "expression": script, "returnByValue": true, "awaitPromise": true, "userGesture": true })).await?;
        if let Some(ex) = r.get("exceptionDetails") {
            let text = ex["exception"]["description"]
                .as_str()
                .or(ex["text"].as_str())
                .unwrap_or("the step failed");
            return Err(text
                .lines()
                .next()
                .unwrap_or(text)
                .trim_start_matches("Error: ")
                .to_string());
        }
        Ok(r["result"]["value"].clone())
    }

    /// Waits until the page has loaded (or a few seconds pass), then a moment for scripts.
    async fn settle(&mut self) {
        for _ in 0..30 {
            if self
                .eval("document.readyState")
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .as_deref()
                == Some("complete")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    }
}

/// Starts the browser with a DevTools port and returns it with its WebSocket address.
async fn launch(browser: &Path, profile: &Path) -> Result<(tokio::process::Child, String), String> {
    use tokio::io::AsyncBufReadExt as _;
    let mut args = headless_args(profile, "about:blank", 1280, 900, None);
    args.retain(|a| {
        a != "--dump-dom" && a != "about:blank" && !a.starts_with("--virtual-time-budget")
    });
    args.push("--remote-debugging-port=0".into());
    args.push("about:blank".into());
    let mut child = tokio::process::Command::new(browser)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("The browser could not start: {e}"))?;
    let stderr = child.stderr.take().ok_or("no browser output")?;
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    // What the browser said, to explain a start that fails.
    let mut said: Vec<String> = Vec::new();
    // A cold start on a busy computer can take a while.
    let waited = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = line.split("DevTools listening on ").nth(1) {
                return Some(url.trim().to_string());
            }
            said.push(line);
            if said.len() > 8 {
                said.remove(0);
            }
        }
        // The browser closed its output: it exited.
        None
    })
    .await;
    let found = match waited {
        Ok(Some(url)) => url,
        outcome => {
            let why = if outcome.is_err() {
                "within a minute"
            } else {
                "before it exited"
            };
            let tail = said.join(" | ");
            return Err(format!(
                "The browser did not open its DevTools port {why}.{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(" It said: {tail}")
                }
            ));
        }
    };
    // Keep reading its output so it never blocks on a full pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Ok((child, found))
}

/// Runs `browser_act`.
pub async fn act(
    config_dir: &Path,
    conversation_id: &str,
    agent: Option<&str>,
    args: &Value,
) -> Outcome {
    let err = |text: String| Outcome {
        text,
        is_error: true,
    };
    let Some(browser) = find_browser() else {
        return err("No Chrome, Chromium, Edge or Brave is installed.".into());
    };
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")) {
        return err("Give a full http(s) address.".into());
    }
    let steps: Vec<Value> = args
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if steps.len() > MAX_STEPS {
        return err(format!("At most {MAX_STEPS} steps at once."));
    }
    let profile = profile_dir(config_dir, agent);
    let _ = std::fs::create_dir_all(&profile);
    let (mut child, ws_url) = match launch(&browser, &profile).await {
        Ok(x) => x,
        Err(e) => return err(e),
    };
    let result = async {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url.as_str()).await.map_err(|e| e.to_string())?;
        let mut cdp = Cdp { ws, next: 0, session: None };
        let target = cdp.call("Target.createTarget", json!({ "url": "about:blank" })).await?;
        let target_id = target["targetId"].as_str().ok_or("no page")?.to_string();
        let attached = cdp.call("Target.attachToTarget", json!({ "targetId": target_id, "flatten": true })).await?;
        cdp.session = attached["sessionId"].as_str().map(str::to_string);
        cdp.call("Page.enable", json!({})).await?;
        cdp.call("Emulation.setDeviceMetricsOverride", json!({ "width": 1280, "height": 900, "deviceScaleFactor": 1, "mobile": false })).await?;
        cdp.call("Page.navigate", json!({ "url": url })).await?;
        cdp.settle().await;
        let mut log: Vec<String> = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            let done = if let Some(ms) = step.get("wait").and_then(Value::as_u64) {
                tokio::time::sleep(std::time::Duration::from_millis(ms.min(10_000))).await;
                Ok(format!("waited {ms} ms"))
            } else if let Some(next) = step.get("goto").and_then(Value::as_str) {
                cdp.call("Page.navigate", json!({ "url": next })).await.map(|_| format!("went to {next}"))
            } else {
                match step_js(step) {
                    Ok(script) => cdp.eval(&script).await.map(|v| v.as_str().unwrap_or("done").to_string()),
                    Err(e) => Err(e),
                }
            };
            match done {
                Ok(what) => log.push(format!("{}. {what}", i + 1)),
                Err(e) => {
                    log.push(format!("{}. failed: {e}", i + 1));
                    break;
                }
            }
            cdp.settle().await;
        }
        let shot = cdp.call("Page.captureScreenshot", json!({ "format": "png" })).await?;
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, shot["data"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
        let page = cdp.eval("JSON.stringify({ url: location.href, title: document.title, text: document.body ? document.body.innerText : '' })").await?;
        let mut page: Value = serde_json::from_str(page.as_str().unwrap_or("{}")).unwrap_or(Value::Null);
        if let Ok(controls) = cdp.eval(CONTROLS_JS).await {
            page["controls"] = serde_json::from_str(controls.as_str().unwrap_or("[]")).unwrap_or(Value::Null);
        }
        let _ = cdp.call("Browser.close", json!({})).await;
        Ok::<_, String>((log, png, page))
    }
    .await;
    let _ = child.kill().await;
    let (log, png, page) = match result {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    let dir = match crate::chat::sandbox::workspace(config_dir, conversation_id) {
        Ok(d) => d.join("images"),
        Err(e) => return err(e),
    };
    let _ = std::fs::create_dir_all(&dir);
    let name = format!(
        "page-{}.png",
        chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
    );
    if let Err(e) = std::fs::write(dir.join(&name), &png) {
        return err(e.to_string());
    }
    let text: String = page["text"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(MAX_TEXT)
        .collect();
    let failed = log.iter().any(|l| l.contains("failed:"));
    Outcome {
        text: json!({ "file": format!("images/{name}"), "url": page["url"], "title": page["title"], "steps": log, "text": text, "controls": page["controls"], "hint": if failed { "A step failed: pick a target from `controls` and try again." } else { "" } }).to_string(),
        is_error: failed && steps.len() == 1,
    }
}

#[cfg(test)]
mod act_tests {
    use super::*;

    #[test]
    fn steps_become_scripts() {
        assert!(step_js(&json!({"click": "text=Sign in"}))
            .unwrap()
            .contains("\"text=Sign in\""));
        assert!(step_js(&json!({"type": "hello \"world\"", "into": "#q"}))
            .unwrap()
            .contains("\"hello \\\"world\\\"\""));
        assert!(step_js(&json!({"press": "Enter"}))
            .unwrap()
            .contains("requestSubmit"));
        assert!(step_js(&json!({"press": "Delete"})).is_err());
        assert!(step_js(&json!({"dance": 1})).is_err());
    }

    /// A browser that exits without opening its DevTools port is reported with what it said.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_browser_that_exits_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake-browser");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho 'Missing X server or $DISPLAY' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let err = launch(&fake, dir.path()).await.unwrap_err();
        assert!(
            err.contains("before it exited") && err.contains("Missing X server"),
            "{err}"
        );
    }

    /// Runs only where a browser is installed: fills a form and reads what its script shows.
    #[tokio::test]
    async fn a_real_browser_fills_a_form() {
        if find_browser().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("form.html");
        std::fs::write(&page, "<title>Form</title><form onsubmit=\"event.preventDefault();document.getElementById('out').textContent='Hello, '+document.getElementById('name').value\"><input id=name><button>Greet</button></form><p id=out></p><a href='#' onclick=\"document.getElementById('out').textContent+=' (and clicked)'\">More</a>").unwrap();
        std::fs::write(
            &page,
            std::fs::read_to_string(&page)
                .unwrap()
                .replace("<input id=name>", "<input id=name placeholder='Your name'>"),
        )
        .unwrap();
        let out = act(dir.path(), "c1", Some("a1"), &json!({ "url": format!("file://{}", page.display()), "steps": [{"type": "Ana", "into": "text=Your name"}, {"press": "Enter"}, {"click": "text=More"}] })).await;
        assert!(!out.is_error, "{}", out.text);
        let v: Value = serde_json::from_str(&out.text).unwrap();
        assert!(
            v["text"]
                .as_str()
                .unwrap()
                .contains("Hello, Ana (and clicked)"),
            "{v}"
        );
        assert_eq!(v["steps"].as_array().unwrap().len(), 3);
        let controls = v["controls"].to_string();
        assert!(
            controls.contains("#name") && controls.contains("text=Greet"),
            "{controls}"
        );
        let shot = crate::chat::sandbox::workspace(dir.path(), "c1")
            .unwrap()
            .join(v["file"].as_str().unwrap());
        assert!(std::fs::read(shot).unwrap().starts_with(b"\x89PNG"));
    }
}
