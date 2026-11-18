//! The built-in model runtime (Osaurus's and LM Studio's local models, OpenBot's on-device
//! dictation): chat models and Whisper speech-to-text run inside Lumo, on this computer's CPU,
//! with Hugging Face's `candle` (pure Rust). Nothing to install: a model is downloaded once from
//! Hugging Face (Settings → Providers → Built-in models) and works offline after that.
//!
//! - Chat models (GGUF, 4-bit): Qwen 2.5 and Llama 3.2. They appear in the model picker as
//!   `local/<id>`. The engine reaches them through a small OpenAI-compatible endpoint on
//!   127.0.0.1, started on first use, so streaming and every other path work as for any provider.
//! - Speech: Whisper (tiny to large-v3-turbo) as the "Built-in" speech-to-text engine, for the
//!   composer's microphone and for dictation anywhere.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use candle_core::{Device, Tensor};
use serde::Serialize;
use serde_json::{json, Value};

pub const VENDOR: &str = "local";

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Chat,
    Speech,
    /// Turns text into vectors, for semantic library search.
    Embedding,
}

/// A model that can be downloaded: its files on Hugging Face (`repo`, path) saved as names here.
pub struct Entry {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub size_mb: u32,
    pub note: &'static str,
    pub files: &'static [(&'static str, &'static str, &'static str)],
}

pub const CATALOG: &[Entry] = &[
    Entry { id: "qwen2.5-0.5b", name: "Qwen 2.5 0.5B", kind: Kind::Chat, size_mb: 500, note: "Tiny and quick: short answers, rewrites, titles. Any computer.", files: &[("Qwen/Qwen2.5-0.5B-Instruct-GGUF", "qwen2.5-0.5b-instruct-q4_k_m.gguf", "model.gguf"), ("Qwen/Qwen2.5-0.5B-Instruct", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "qwen2.5-1.5b", name: "Qwen 2.5 1.5B", kind: Kind::Chat, size_mb: 1_100, note: "Good everyday answers on most laptops.", files: &[("Qwen/Qwen2.5-1.5B-Instruct-GGUF", "qwen2.5-1.5b-instruct-q4_k_m.gguf", "model.gguf"), ("Qwen/Qwen2.5-1.5B-Instruct", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "qwen2.5-3b", name: "Qwen 2.5 3B", kind: Kind::Chat, size_mb: 2_000, note: "Better reasoning and writing; 8 GB of memory or more.", files: &[("Qwen/Qwen2.5-3B-Instruct-GGUF", "qwen2.5-3b-instruct-q4_k_m.gguf", "model.gguf"), ("Qwen/Qwen2.5-3B-Instruct", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "qwen2.5-7b", name: "Qwen 2.5 7B", kind: Kind::Chat, size_mb: 4_700, note: "The strongest here; 16 GB of memory, slower on a laptop.", files: &[("bartowski/Qwen2.5-7B-Instruct-GGUF", "Qwen2.5-7B-Instruct-Q4_K_M.gguf", "model.gguf"), ("Qwen/Qwen2.5-7B-Instruct", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "llama-3.2-1b", name: "Llama 3.2 1B", kind: Kind::Chat, size_mb: 810, note: "Meta's small model; quick, English first.", files: &[("bartowski/Llama-3.2-1B-Instruct-GGUF", "Llama-3.2-1B-Instruct-Q4_K_M.gguf", "model.gguf"), ("unsloth/Llama-3.2-1B-Instruct", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "llama-3.2-3b", name: "Llama 3.2 3B", kind: Kind::Chat, size_mb: 2_000, note: "Meta's mid-size model; 8 GB of memory or more.", files: &[("bartowski/Llama-3.2-3B-Instruct-GGUF", "Llama-3.2-3B-Instruct-Q4_K_M.gguf", "model.gguf"), ("unsloth/Llama-3.2-3B-Instruct", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "minilm-l6", name: "MiniLM L6 (embeddings)", kind: Kind::Embedding, size_mb: 90, note: "Semantic search for your libraries, on this computer (English first).", files: &[("sentence-transformers/all-MiniLM-L6-v2", "model.safetensors", "model.safetensors"), ("sentence-transformers/all-MiniLM-L6-v2", "config.json", "config.json"), ("sentence-transformers/all-MiniLM-L6-v2", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "whisper-tiny", name: "Whisper tiny", kind: Kind::Speech, size_mb: 150, note: "Fast dictation; fine for clear speech.", files: &[("openai/whisper-tiny", "model.safetensors", "model.safetensors"), ("openai/whisper-tiny", "config.json", "config.json"), ("openai/whisper-tiny", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "whisper-base", name: "Whisper base", kind: Kind::Speech, size_mb: 290, note: "A good balance for dictation.", files: &[("openai/whisper-base", "model.safetensors", "model.safetensors"), ("openai/whisper-base", "config.json", "config.json"), ("openai/whisper-base", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "whisper-small", name: "Whisper small", kind: Kind::Speech, size_mb: 970, note: "More accurate, other languages too; slower.", files: &[("openai/whisper-small", "model.safetensors", "model.safetensors"), ("openai/whisper-small", "config.json", "config.json"), ("openai/whisper-small", "tokenizer.json", "tokenizer.json")] },
    Entry { id: "whisper-large-v3-turbo", name: "Whisper large-v3 turbo", kind: Kind::Speech, size_mb: 1_620, note: "The most accurate; best with a fast computer.", files: &[("openai/whisper-large-v3-turbo", "model.safetensors", "model.safetensors"), ("openai/whisper-large-v3-turbo", "config.json", "config.json"), ("openai/whisper-large-v3-turbo", "tokenizer.json", "tokenizer.json")] },
];

pub fn entry(id: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.id == id)
}

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Where models are kept: the app's local data folder (not the settings folder, so backups and
/// sync leave gigabytes of weights alone). Set once at start.
pub fn init(local_data_dir: &Path) {
    let _ = ROOT.set(local_data_dir.join("models"));
}

pub fn root() -> Result<PathBuf, String> {
    ROOT.get().cloned().ok_or_else(|| "The model folder is not set up yet.".into())
}

fn model_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    entry(id).ok_or_else(|| format!("Unknown model `{id}`."))?;
    Ok(root.join(id))
}

pub fn installed(root: &Path, id: &str) -> bool {
    model_dir(root, id).is_ok_and(|d| d.join(".complete").exists())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub size_mb: u32,
    pub note: String,
    pub installed: bool,
    pub downloading: bool,
}

pub fn list(root: &Path) -> Vec<View> {
    CATALOG
        .iter()
        .map(|e| View { id: e.id.into(), name: e.name.into(), kind: e.kind, size_mb: e.size_mb, note: e.note.into(), installed: installed(root, e.id), downloading: downloads().lock().is_ok_and(|d| d.contains(&e.id.to_string())) })
        .collect()
}

/// The chat models downloaded, for the model picker.
pub fn models(root: &Path) -> Vec<super::providers::ModelView> {
    CATALOG
        .iter()
        .filter(|e| e.kind == Kind::Chat && installed(root, e.id))
        .map(|e| super::providers::ModelView { reference: format!("{VENDOR}/{}", e.id), vendor: VENDOR.into(), provider_label: "Built-in (on this computer)".into(), id: e.id.into(), name: e.name.into(), reasoning: false, tools: false, context_window: Some(CONTEXT as u32) })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Downloads

fn downloads() -> &'static Mutex<Vec<String>> {
    static D: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Vec::new()))
}

fn cancelled() -> &'static Mutex<Vec<String>> {
    static C: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn cancel(id: &str) {
    if let Ok(mut c) = cancelled().lock() {
        c.push(id.to_string());
    }
}

/// Where a file comes from. `LUMO_MODELS_URL` points at a mirror with the same layout.
fn file_url(repo: &str, path: &str) -> String {
    let base = std::env::var("LUMO_MODELS_URL").unwrap_or_else(|_| "https://huggingface.co".into());
    format!("{}/{repo}/resolve/main/{path}", base.trim_end_matches('/'))
}

/// Downloads a model's files, reporting `(bytes done, bytes in all)` as it goes.
pub async fn download(root: &Path, id: &str, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    use futures::StreamExt as _;
    let e = entry(id).ok_or_else(|| format!("Unknown model `{id}`."))?;
    {
        let mut d = downloads().lock().map_err(|e| e.to_string())?;
        if d.iter().any(|x| x == id) {
            return Err("It is already downloading.".into());
        }
        d.push(id.to_string());
    }
    let result = async {
        let dir = model_dir(root, id)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
        // Sizes first, for one progress bar over all files.
        let mut sizes = Vec::new();
        for (repo, path, _) in e.files {
            let head = client.head(file_url(repo, path)).send().await.map_err(|err| format!("Hugging Face did not answer: {err}"))?;
            sizes.push(head.headers().get("x-linked-size").or_else(|| head.headers().get("content-length")).and_then(|v| v.to_str().ok()?.parse::<u64>().ok()).unwrap_or(0));
        }
        let total: u64 = sizes.iter().sum();
        let mut done = 0u64;
        for (i, (repo, path, name)) in e.files.iter().enumerate() {
            let target = dir.join(name);
            if target.exists() && std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0) == sizes[i] && sizes[i] > 0 {
                done += sizes[i];
                progress(done, total);
                continue;
            }
            let r = client.get(file_url(repo, path)).send().await.map_err(|err| format!("The download failed: {err}"))?;
            if !r.status().is_success() {
                return Err(format!("Hugging Face answered {} for {path}.", r.status()));
            }
            let part = dir.join(format!("{name}.part"));
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let mut stream = r.bytes_stream();
            let mut last = std::time::Instant::now();
            while let Some(chunk) = stream.next().await {
                if cancelled().lock().is_ok_and(|c| c.iter().any(|x| x == id)) {
                    drop(out);
                    let _ = std::fs::remove_file(&part);
                    return Err("Download cancelled.".into());
                }
                let chunk = chunk.map_err(|err| format!("The download stopped: {err}"))?;
                out.write_all(&chunk).map_err(|e| e.to_string())?;
                done += chunk.len() as u64;
                if last.elapsed().as_millis() > 250 {
                    last = std::time::Instant::now();
                    progress(done, total);
                }
            }
            out.flush().map_err(|e| e.to_string())?;
            drop(out);
            std::fs::rename(&part, &target).map_err(|e| e.to_string())?;
        }
        std::fs::write(dir.join(".complete"), b"").map_err(|e| e.to_string())?;
        progress(total, total);
        Ok(())
    }
    .await;
    if let Ok(mut d) = downloads().lock() {
        d.retain(|x| x != id);
    }
    if let Ok(mut c) = cancelled().lock() {
        c.retain(|x| x != id);
    }
    result
}

pub fn remove(root: &Path, id: &str) -> Result<(), String> {
    unload(id);
    match std::fs::remove_dir_all(model_dir(root, id)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Chat models

/// Tokens of context kept: the prompt is cut from the oldest messages to fit.
const CONTEXT: usize = 8_192;
/// The longest answer, in tokens, unless asked for less.
const MAX_ANSWER: usize = 2_048;

enum Weights {
    Qwen2(candle_transformers::models::quantized_qwen2::ModelWeights),
    Llama(candle_transformers::models::quantized_llama::ModelWeights),
}

impl Weights {
    fn forward(&mut self, x: &Tensor, pos: usize) -> candle_core::Result<Tensor> {
        match self {
            Weights::Qwen2(m) => m.forward(x, pos),
            Weights::Llama(m) => m.forward(x, pos),
        }
    }
    fn clear(&mut self) {
        match self {
            Weights::Qwen2(m) => m.clear_kv_cache(),
            Weights::Llama(m) => m.clear_kv_cache(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Template {
    ChatMl,
    Llama3,
}

struct Loaded {
    id: String,
    weights: Weights,
    tokenizer: tokenizers::Tokenizer,
    template: Template,
    stop: Vec<u32>,
}

/// The model in memory. A fair lock: waiting requests run in the order they came.
fn loaded() -> &'static parking_lot::FairMutex<Option<Loaded>> {
    static L: OnceLock<parking_lot::FairMutex<Option<Loaded>>> = OnceLock::new();
    L.get_or_init(|| parking_lot::FairMutex::new(None))
}

/// Frees a model's memory (when it is deleted).
pub fn unload(id: &str) {
    if let Some(mut l) = loaded().try_lock() {
        if l.as_ref().is_some_and(|m| m.id == id) {
            *l = None;
        }
    }
}

fn load(root: &Path, id: &str) -> Result<Loaded, String> {
    if !installed(root, id) {
        return Err(format!("Download {} first (Settings → Providers → Built-in models).", entry(id).map_or(id, |e| e.name)));
    }
    let dir = model_dir(root, id)?;
    let mut file = std::fs::File::open(dir.join("model.gguf")).map_err(|e| e.to_string())?;
    let content = candle_core::quantized::gguf_file::Content::read(&mut file).map_err(|e| format!("The model file could not be read: {e}"))?;
    let arch = content.metadata.get("general.architecture").and_then(|v| v.to_string().ok()).cloned().unwrap_or_default();
    let device = Device::Cpu;
    let (weights, template) = match arch.as_str() {
        "qwen2" => (Weights::Qwen2(candle_transformers::models::quantized_qwen2::ModelWeights::from_gguf(content, &mut file, &device).map_err(|e| e.to_string())?), Template::ChatMl),
        "llama" => (Weights::Llama(candle_transformers::models::quantized_llama::ModelWeights::from_gguf(content, &mut file, &device).map_err(|e| e.to_string())?), Template::Llama3),
        other => return Err(format!("Models of the `{other}` kind are not supported yet.")),
    };
    let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| format!("The tokenizer could not be read: {e}"))?;
    let stop = ["<|im_end|>", "<|endoftext|>", "<|eot_id|>", "<|end_of_text|>", "<|eom_id|>"].iter().filter_map(|t| tokenizer.token_to_id(t)).collect();
    Ok(Loaded { id: id.to_string(), weights, tokenizer, template, stop })
}

/// The text of an OpenAI-style message's content: a string, or the text of its parts.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Messages as `(role, text)`: tool results are told to the model as text.
pub fn turns(messages: &[Value]) -> Vec<(String, String)> {
    messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
            let text = content_text(m.get("content").unwrap_or(&Value::Null));
            match role {
                "system" | "developer" => Some(("system".to_string(), text)),
                "assistant" => (!text.trim().is_empty()).then(|| ("assistant".to_string(), text)),
                "tool" => Some(("user".to_string(), format!("Tool result:\n{text}"))),
                _ => Some(("user".to_string(), text)),
            }
        })
        .collect()
}

/// The prompt in the model's chat format, ending where the assistant's answer starts.
pub fn render(template: Template, turns: &[(String, String)]) -> String {
    let mut out = String::new();
    match template {
        Template::ChatMl => {
            for (role, text) in turns {
                out.push_str(&format!("<|im_start|>{role}\n{text}<|im_end|>\n"));
            }
            out.push_str("<|im_start|>assistant\n");
        }
        Template::Llama3 => {
            out.push_str("<|begin_of_text|>");
            for (role, text) in turns {
                out.push_str(&format!("<|start_header_id|>{role}<|end_header_id|>\n\n{text}<|eot_id|>"));
            }
            out.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
        }
    }
    out
}

pub struct Params {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<usize>,
}

pub struct Usage {
    pub prompt: usize,
    pub completion: usize,
}

/// Writes an answer to `messages`, giving each new piece of text to `on_text`; `on_text`
/// returning false stops it (the reader went away). Runs on the calling thread.
pub fn generate(root: &Path, id: &str, messages: &[Value], params: &Params, mut on_text: impl FnMut(&str) -> bool) -> Result<Usage, String> {
    let mut guard = loaded().lock();
    if guard.as_ref().is_none_or(|m| m.id != id) {
        *guard = None;
        *guard = Some(load(root, id)?);
    }
    let m = guard.as_mut().ok_or("The model is not loaded.")?;
    let mut turns = turns(messages);
    // Oldest messages go first (the system prompt stays) until the prompt fits.
    let budget = CONTEXT - params.max_tokens.unwrap_or(MAX_ANSWER).min(MAX_ANSWER);
    let mut prompt = m.tokenizer.encode(render(m.template, &turns), false).map_err(|e| e.to_string())?.get_ids().to_vec();
    while prompt.len() > budget && turns.len() > 2 {
        let first = usize::from(turns[0].0 == "system");
        turns.remove(first);
        prompt = m.tokenizer.encode(render(m.template, &turns), false).map_err(|e| e.to_string())?.get_ids().to_vec();
    }
    if prompt.len() > budget {
        prompt = prompt[prompt.len() - budget..].to_vec();
    }
    let device = Device::Cpu;
    let temperature = params.temperature.unwrap_or(0.7);
    let mut sampler = candle_transformers::generation::LogitsProcessor::new(rand_seed(), (temperature > 0.0).then_some(temperature), params.top_p);
    m.weights.clear();
    let prefill_started = std::time::Instant::now();
    let input = Tensor::new(prompt.as_slice(), &device).and_then(|t| t.unsqueeze(0)).map_err(|e| e.to_string())?;
    let mut logits = m.weights.forward(&input, 0).and_then(|l| l.squeeze(0)).map_err(|e| e.to_string())?;
    log::info!("built-in model {id}: read {} prompt tokens in {:?}", prompt.len(), prefill_started.elapsed());
    let mut answer: Vec<u32> = Vec::new();
    let mut shown = 0usize;
    let limit = params.max_tokens.unwrap_or(MAX_ANSWER).min(MAX_ANSWER);
    for i in 0..limit {
        let recent = &answer[answer.len().saturating_sub(64)..];
        let penalized = candle_transformers::utils::apply_repeat_penalty(&logits, 1.1, recent).map_err(|e| e.to_string())?;
        let next = sampler.sample(&penalized).map_err(|e| e.to_string())?;
        if m.stop.contains(&next) {
            break;
        }
        answer.push(next);
        // Whole characters only: a piece is shown once it decodes cleanly.
        let text = m.tokenizer.decode(&answer, true).map_err(|e| e.to_string())?;
        if text.len() > shown && !text.ends_with('\u{FFFD}') {
            if !on_text(&text[shown..]) {
                // The reader went away (the reply was stopped).
                break;
            }
            shown = text.len();
        }
        let input = Tensor::new(&[next], &device).and_then(|t| t.unsqueeze(0)).map_err(|e| e.to_string())?;
        logits = m.weights.forward(&input, prompt.len() + i).and_then(|l| l.squeeze(0)).map_err(|e| e.to_string())?;
    }
    Ok(Usage { prompt: prompt.len(), completion: answer.len() })
}

fn rand_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(299_792_458, |d| d.as_nanos() as u64)
}

// ---------------------------------------------------------------------------------------------
// The endpoint the engine talks to: OpenAI's chat completions, on 127.0.0.1 only.

static PORT: tokio::sync::OnceCell<u16> = tokio::sync::OnceCell::const_new();

/// The port of the built-in endpoint, started on first use.
pub async fn endpoint() -> Result<u16, String> {
    PORT.get_or_try_init(|| async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let app = axum::Router::new()
            .route("/v1/chat/completions", axum::routing::post(completions))
            .route("/v1/models", axum::routing::get(list_models));
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                log::warn!("the built-in model endpoint stopped: {e}");
            }
        });
        Ok::<u16, String>(port)
    })
    .await
    .copied()
}

async fn list_models() -> axum::Json<Value> {
    let ids: Vec<Value> = root().map(|r| models(&r)).unwrap_or_default().into_iter().map(|m| json!({ "id": m.id, "object": "model", "owned_by": "lumo" })).collect();
    axum::Json(json!({ "object": "list", "data": ids }))
}

fn error_response(status: u16, message: &str) -> axum::response::Response {
    let body = json!({ "error": { "message": message, "type": "invalid_request_error" } }).to_string();
    axum::response::Response::builder().status(status).header("content-type", "application/json").body(axum::body::Body::from(body)).unwrap_or_default()
}

/// Requests waiting for or using the processor. One model runs at a time; the rest wait in
/// arrival order (the model's lock is fair), and past `QUEUE_LIMIT` a request is told to retry.
static QUEUED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub const QUEUE_LIMIT: usize = 8;

/// Holds a place in the queue for as long as it lives.
pub struct Place;

impl Place {
    /// A place, or `None` when the queue is full.
    pub fn take() -> Option<Place> {
        use std::sync::atomic::Ordering;
        let mut now = QUEUED.load(Ordering::SeqCst);
        loop {
            if now >= QUEUE_LIMIT {
                return None;
            }
            match QUEUED.compare_exchange(now, now + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(Place),
                Err(actual) => now = actual,
            }
        }
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        QUEUED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn completions(axum::Json(body): axum::Json<Value>) -> axum::response::Response {
    let Ok(root) = root() else { return error_response(500, "The model folder is not set up.") };
    let Some(place) = Place::take() else {
        let body = json!({ "error": { "message": format!("The built-in runtime is busy ({QUEUE_LIMIT} requests waiting); try again shortly.", ), "type": "rate_limit_error" } }).to_string();
        return axum::response::Response::builder().status(429).header("content-type", "application/json").header("retry-after", "5").body(axum::body::Body::from(body)).unwrap_or_default();
    };
    let id = body.get("model").and_then(Value::as_str).unwrap_or_default().trim_start_matches("local/").to_string();
    if !installed(&root, &id) {
        return error_response(404, &format!("The model `{id}` is not downloaded."));
    }
    let messages = body.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
    let params = Params {
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        max_tokens: body.get("max_completion_tokens").or_else(|| body.get("max_tokens")).and_then(Value::as_u64).map(|n| n as usize),
    };
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    log::info!("built-in model {id}: {} messages, {} characters, {} tools offered, stream {stream}", messages.len(), messages.iter().map(|m| content_text(m.get("content").unwrap_or(&Value::Null)).len()).sum::<usize>(), body.get("tools").and_then(Value::as_array).map_or(0, Vec::len));
    let created = chrono::Utc::now().timestamp();
    let rid = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();
    let model_id = id.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _place = place;
        let tx_text = tx.clone();
        let result = generate(&root, &model_id, &messages, &params, |piece| tx_text.send(Ok(piece.to_string())).is_ok());
        result.inspect_err(|e| {
            let _ = tx.send(Err(e.clone()));
        })
    });
    if !stream {
        let mut rx = rx;
        let mut text = String::new();
        while let Some(piece) = rx.recv().await {
            match piece {
                Ok(p) => text.push_str(&p),
                Err(e) => return error_response(500, &e),
            }
        }
        let usage = match task.await {
            Ok(Ok(u)) => u,
            Ok(Err(e)) => return error_response(500, &e),
            Err(e) => return error_response(500, &e.to_string()),
        };
        let body = json!({
            "id": rid, "object": "chat.completion", "created": created, "model": id,
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": usage.prompt, "completion_tokens": usage.completion, "total_tokens": usage.prompt + usage.completion }
        });
        return axum::response::IntoResponse::into_response(axum::Json(body));
    }
    // Server-sent events, as OpenAI streams them.
    let chunk = move |delta: Value, finish: Option<&str>, usage: Option<Value>| {
        let mut c = json!({ "id": rid, "object": "chat.completion.chunk", "created": created, "model": id, "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] });
        if let Some(u) = usage {
            c["usage"] = u;
            c["choices"] = json!([]);
        }
        format!("data: {c}\n\n")
    };
    let state = (rx, Some(task), false, chunk);
    let events = futures::stream::unfold(state, |(mut rx, task, started, chunk)| async move {
        if !started {
            let first = chunk(json!({ "role": "assistant", "content": "" }), None, None);
            return Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(first)), (rx, task, true, chunk)));
        }
        match rx.recv().await {
            Some(Ok(piece)) => Some((Ok(axum::body::Bytes::from(chunk(json!({ "content": piece }), None, None))), (rx, task, true, chunk))),
            Some(Err(e)) => {
                let err = format!("data: {}\n\ndata: [DONE]\n\n", json!({ "error": { "message": e } }));
                Some((Ok(axum::body::Bytes::from(err)), (rx, None, true, chunk)))
            }
            None => {
                let task = task?;
                let usage = match task.await {
                    Ok(Ok(u)) => json!({ "prompt_tokens": u.prompt, "completion_tokens": u.completion, "total_tokens": u.prompt + u.completion }),
                    _ => json!({ "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }),
                };
                let end = format!("{}{}data: [DONE]\n\n", chunk(json!({}), Some("stop"), None), chunk(json!({}), None, Some(usage)));
                Some((Ok(axum::body::Bytes::from(end)), (rx, None, true, chunk)))
            }
        }
    });
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(axum::body::Body::from_stream(events))
        .unwrap_or_default()
}

/// The system prompt of a built-in model: who it is, today's date, and the user's own
/// instructions; short, because a small model on a processor reads every token.
pub fn brief(instructions: &super::store::Instructions, project: Option<&str>, agent: Option<(&str, &str)>) -> String {
    let mut out = match agent {
        Some((name, text)) => format!("You are {name}, running on the user's own computer. {}", text.trim()),
        None => "You are Lumo, a helpful assistant running on the user's own computer. Answer clearly and briefly; use Markdown when it helps.".to_string(),
    };
    out.push_str(&format!("\nToday is {}.", chrono::Local::now().format("%A, %-d %B %Y")));
    if !instructions.name.trim().is_empty() {
        out.push_str(&format!("\nThe user's name is {}.", instructions.name.trim()));
    }
    if !instructions.tone.trim().is_empty() {
        out.push_str(&format!("\nTone: {}.", instructions.tone.trim()));
    }
    for extra in [instructions.custom.trim(), project.unwrap_or_default().trim()] {
        if !extra.is_empty() {
            out.push_str("\n\n");
            out.push_str(&extra.chars().take(2_000).collect::<String>());
        }
    }
    out
}

/// The model a `local/<id>` reference runs on, through the built-in endpoint.
pub async fn connect(reference: &str) -> Result<rig_core::DynModel<rig_core::operation::Completion>, String> {
    let id = reference.strip_prefix(VENDOR).and_then(|r| r.strip_prefix('/')).ok_or("Not a built-in model.")?;
    let root = root()?;
    if !installed(&root, id) {
        return Err(format!("Download {} first (Settings → Providers → Built-in models).", entry(id).map_or(id, |e| e.name)));
    }
    let port = endpoint().await?;
    let settings = super::providers::ProviderSettings { enabled: true, base_url: Some(format!("http://127.0.0.1:{port}/v1")), ..Default::default() };
    super::providers::connect(&format!("llamacpp/{id}"), &settings, None)
}

// ---------------------------------------------------------------------------------------------
// Speech: Whisper

/// Whisper's language codes, as its `<|xx|>` tokens.
const LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", "yue",
];

/// The mel filter bank Whisper was trained with (librosa's Slaney-style filters), `n_mels`
/// rows of `n_fft / 2 + 1` weights.
pub fn mel_filters(n_mels: usize) -> Vec<f32> {
    let sr = 16_000.0f64;
    let n_fft = 400usize;
    let bins = n_fft / 2 + 1;
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    let hz_to_mel = |f: f64| if f < min_log_hz { f / f_sp } else { min_log_mel + (f / min_log_hz).ln() / logstep };
    let mel_to_hz = |m: f64| if m < min_log_mel { m * f_sp } else { min_log_hz * (logstep * (m - min_log_mel)).exp() };
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..n_mels + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64)).collect();
    let fft_f: Vec<f64> = (0..bins).map(|i| sr / 2.0 * i as f64 / (bins - 1) as f64).collect();
    let mut out = vec![0f32; n_mels * bins];
    for m in 0..n_mels {
        let enorm = 2.0 / (mel_f[m + 2] - mel_f[m]);
        for (k, f) in fft_f.iter().enumerate() {
            let lower = (f - mel_f[m]) / (mel_f[m + 1] - mel_f[m]);
            let upper = (mel_f[m + 2] - f) / (mel_f[m + 2] - mel_f[m + 1]);
            out[m * bins + k] = (lower.min(upper).max(0.0) * enorm) as f32;
        }
    }
    out
}

/// Mono 16 kHz samples from a WAV file (16-bit, 24-bit, 32-bit or float), mixed and resampled.
pub fn wav_samples(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut channels, mut rate, mut bits, mut format) = (1usize, 16_000usize, 16usize, 1u16);
    let mut data: &[u8] = &[];
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len = u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        let body = &bytes[i + 8..(i + 8 + len).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            format = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]).max(1) as usize;
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
            bits = u16::from_le_bytes([body[14], body[15]]) as usize;
            if format == 0xFFFE && body.len() >= 26 {
                format = u16::from_le_bytes([body[24], body[25]]);
            }
        } else if id == b"data" {
            data = body;
        }
        i += 8 + len + (len & 1);
    }
    let width = bits / 8;
    if width == 0 || data.is_empty() {
        return Err("The recording is empty.".into());
    }
    let frames = data.len() / (width * channels);
    let sample = |j: usize| -> f32 {
        let b = &data[j * width..j * width + width];
        match (format, bits) {
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (_, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            (_, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            (_, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            (_, 8) => (b[0] as f32 - 128.0) / 128.0,
            _ => 0.0,
        }
    };
    let mono: Vec<f32> = (0..frames).map(|f| (0..channels).map(|c| sample(f * channels + c)).sum::<f32>() / channels as f32).collect();
    Ok(resample(&mono, rate, 16_000))
}

/// Linear resampling, enough for speech.
pub fn resample(samples: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let n = (samples.len() as f64 * to as f64 / from as f64) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let j = pos as usize;
            let t = (pos - j as f64) as f32;
            let a = samples[j.min(samples.len() - 1)];
            let b = samples[(j + 1).min(samples.len() - 1)];
            a + (b - a) * t
        })
        .collect()
}

/// Samples from any recording: WAV directly, other formats through ffmpeg when installed.
pub fn audio_samples(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if let Ok(s) = wav_samples(bytes) {
        return Ok(s);
    }
    let input = std::env::temp_dir().join(format!("lumo-audio-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&input, bytes).map_err(|e| e.to_string())?;
    let out = std::process::Command::new("ffmpeg")
        .args(["-loglevel", "error", "-i"])
        .arg(&input)
        .args(["-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
        .output();
    let _ = std::fs::remove_file(&input);
    let out = out.map_err(|_| "The built-in speech model reads WAV recordings; install ffmpeg for other formats.".to_string())?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err("The recording could not be read.".into());
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

struct Speech {
    id: String,
    model: candle_transformers::models::whisper::model::Whisper,
    tokenizer: tokenizers::Tokenizer,
    filters: Vec<f32>,
}

fn speech() -> &'static Mutex<Option<Speech>> {
    static S: OnceLock<Mutex<Option<Speech>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

/// Transcribes 16 kHz mono samples with a downloaded Whisper model. `language` is a code like
/// `en`, or empty to let the model tell.
pub fn transcribe(root: &Path, id: &str, samples: &[f32], language: &str) -> Result<String, String> {
    use candle_core::IndexOp as _;
    use candle_transformers::models::whisper as w;
    if !installed(root, id) {
        return Err(format!("Download {} first (Settings → Providers → Built-in models).", entry(id).map_or(id, |e| e.name)));
    }
    if samples.len() < w::SAMPLE_RATE / 10 {
        return Ok(String::new());
    }
    let mut guard = speech().lock().map_err(|e| e.to_string())?;
    if guard.as_ref().is_none_or(|s| s.id != id) {
        *guard = None;
        let dir = model_dir(root, id)?;
        let config: w::Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        // SAFETY: the file is ours, downloaded and not changed while it is mapped.
        let vb = unsafe { candle_nn::VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], w::DTYPE, &Device::Cpu) }.map_err(|e| e.to_string())?;
        let filters = mel_filters(config.num_mel_bins);
        let model = w::model::Whisper::load(&vb, config).map_err(|e| e.to_string())?;
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
        *guard = Some(Speech { id: id.to_string(), model, tokenizer, filters });
    }
    let s = guard.as_mut().ok_or("The speech model is not loaded.")?;
    let token = |t: &str| s.tokenizer.token_to_id(t).ok_or_else(|| format!("The tokenizer has no {t}."));
    let (sot, transcribe, no_ts, eot) = (token(w::SOT_TOKEN)?, token(w::TRANSCRIBE_TOKEN)?, token(w::NO_TIMESTAMPS_TOKEN)?, token(w::EOT_TOKEN)?);
    let n_mels = s.model.config.num_mel_bins;
    let mel = w::audio::pcm_to_mel(&s.model.config, samples, &s.filters);
    let mel = Tensor::from_vec(mel.clone(), (1, n_mels, mel.len() / n_mels), &Device::Cpu).map_err(|e| e.to_string())?;
    let frames = samples.len() / w::HOP_LENGTH;
    let vocab = s.model.config.vocab_size;
    // Only words and the end: no timestamps or other special tokens.
    let mut mask = vec![0f32; vocab];
    for (i, m) in mask.iter_mut().enumerate() {
        if (i as u32 > eot) || s.model.config.suppress_tokens.contains(&(i as u32)) {
            *m = f32::NEG_INFINITY;
        }
    }
    let mask = Tensor::new(mask.as_slice(), &Device::Cpu).map_err(|e| e.to_string())?;
    let mut text = String::new();
    let mut seek = 0;
    let max_tokens = s.model.config.max_target_positions / 2;
    // Whisper reads 30-second windows; the mel is padded with silence, so the last one is full too.
    let (_, _, padded) = mel.dims3().map_err(|e| e.to_string())?;
    while seek < frames {
        let size = (padded - seek).min(w::N_FRAMES);
        let segment = mel.narrow(2, seek, size).map_err(|e| e.to_string())?;
        seek += w::N_FRAMES;
        let features = s.model.encoder.forward(&segment, true).map_err(|e| e.to_string())?;
        // The language: the one asked for, else the one the model finds likeliest.
        let lang = match language.trim() {
            l if !l.is_empty() => s.tokenizer.token_to_id(&format!("<|{l}|>")),
            _ => {
                let ids: Vec<(u32, &str)> = LANGUAGES.iter().filter_map(|l| s.tokenizer.token_to_id(&format!("<|{l}|>")).map(|t| (t, *l))).collect();
                if ids.is_empty() {
                    None
                } else {
                    let input = Tensor::new(&[[sot]], &Device::Cpu).map_err(|e| e.to_string())?;
                    let ys = s.model.decoder.forward(&input, &features, true).map_err(|e| e.to_string())?;
                    let logits = s.model.decoder.final_linear(&ys.i((..1, 0..1)).map_err(|e| e.to_string())?).and_then(|l| l.i(0)?.i(0)).map_err(|e| e.to_string())?;
                    let logits: Vec<f32> = logits.to_vec1().map_err(|e| e.to_string())?;
                    ids.iter().max_by(|a, b| logits[a.0 as usize].total_cmp(&logits[b.0 as usize])).map(|(t, _)| *t)
                }
            }
        };
        let mut tokens = vec![sot];
        tokens.extend(lang);
        tokens.push(transcribe);
        tokens.push(no_ts);
        let start = tokens.len();
        for i in 0..max_tokens {
            let input = Tensor::new(tokens.as_slice(), &Device::Cpu).and_then(|t| t.unsqueeze(0)).map_err(|e| e.to_string())?;
            let ys = s.model.decoder.forward(&input, &features, i == 0).map_err(|e| e.to_string())?;
            let (_, len, _) = ys.dims3().map_err(|e| e.to_string())?;
            let logits = s.model.decoder.final_linear(&ys.i((..1, len - 1..)).map_err(|e| e.to_string())?).and_then(|l| l.i(0)?.i(0)).map_err(|e| e.to_string())?;
            let logits = logits.broadcast_add(&mask).map_err(|e| e.to_string())?;
            let next = logits.argmax(0).and_then(|t| t.to_scalar::<u32>()).map_err(|e| e.to_string())?;
            if next == eot {
                break;
            }
            tokens.push(next);
            // A small model can loop on a phrase: stop when the last stretch repeats the one before.
            let said = &tokens[start..];
            if (4..=24).any(|n| said.len() >= 3 * n && said[said.len() - n..] == said[said.len() - 2 * n..said.len() - n] && said[said.len() - 2 * n..said.len() - n] == said[said.len() - 3 * n..said.len() - 2 * n]) {
                break;
            }
        }
        let piece = s.tokenizer.decode(&tokens[start..], true).map_err(|e| e.to_string())?;
        let piece = piece.trim();
        if !piece.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(piece);
        }
    }
    s.model.reset_kv_cache();
    Ok(text)
}

// ---------------------------------------------------------------------------------------------
// Embeddings: a sentence-transformer (BERT) for semantic library search

struct Embedder {
    id: String,
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
}

fn embedder() -> &'static Mutex<Option<Embedder>> {
    static E: OnceLock<Mutex<Option<Embedder>>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(None))
}

/// Passages embedded per forward pass.
const EMBED_BATCH: usize = 16;

/// Unit-length vectors for `texts`, in order: the mean of the token vectors (as
/// sentence-transformers does), normalized so a dot product is the cosine.
pub fn embed_texts(root: &Path, id: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    use candle_transformers::models::bert;
    if !installed(root, id) || entry(id).is_none_or(|e| e.kind != Kind::Embedding) {
        return Err(format!("Download {} first (Settings → Providers → Built-in models).", entry(id).map_or(id, |e| e.name)));
    }
    let mut guard = embedder().lock().map_err(|e| e.to_string())?;
    if guard.as_ref().is_none_or(|e| e.id != id) {
        *guard = None;
        let dir = model_dir(root, id)?;
        let config: bert::Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        // SAFETY: the file is ours, downloaded and not changed while it is mapped.
        let vb = unsafe { candle_nn::VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], bert::DTYPE, &Device::Cpu) }.map_err(|e| e.to_string())?;
        let model = bert::BertModel::load(vb, &config).map_err(|e| e.to_string())?;
        let mut tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
        tokenizer.with_padding(Some(tokenizers::PaddingParams { strategy: tokenizers::PaddingStrategy::BatchLongest, ..Default::default() }));
        tokenizer.with_truncation(Some(tokenizers::TruncationParams { max_length: 256, ..Default::default() })).map_err(|e| e.to_string())?;
        *guard = Some(Embedder { id: id.to_string(), model, tokenizer });
    }
    let e = guard.as_ref().ok_or("The embedding model is not loaded.")?;
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(EMBED_BATCH) {
        let encodings = e.tokenizer.encode_batch(chunk.to_vec(), true).map_err(|err| err.to_string())?;
        let ids: Vec<Tensor> = encodings.iter().map(|enc| Tensor::new(enc.get_ids(), &Device::Cpu)).collect::<Result<_, _>>().map_err(|err| err.to_string())?;
        let masks: Vec<Tensor> = encodings.iter().map(|enc| Tensor::new(enc.get_attention_mask(), &Device::Cpu)).collect::<Result<_, _>>().map_err(|err| err.to_string())?;
        let ids = Tensor::stack(&ids, 0).map_err(|err| err.to_string())?;
        let mask = Tensor::stack(&masks, 0).map_err(|err| err.to_string())?;
        let types = ids.zeros_like().map_err(|err| err.to_string())?;
        let hidden = e.model.forward(&ids, &types, Some(&mask)).map_err(|err| err.to_string())?;
        // Mean over the real tokens only.
        let mask_f = mask.to_dtype(candle_core::DType::F32).and_then(|m| m.unsqueeze(2)).map_err(|err| err.to_string())?;
        let summed = hidden.broadcast_mul(&mask_f).and_then(|h| h.sum(1)).map_err(|err| err.to_string())?;
        let counts = mask_f.sum(1).map_err(|err| err.to_string())?;
        let mean = summed.broadcast_div(&counts).map_err(|err| err.to_string())?;
        let norm = mean.sqr().and_then(|s| s.sum_keepdim(1)).and_then(|s| s.sqrt()).map_err(|err| err.to_string())?;
        let unit = mean.broadcast_div(&norm).map_err(|err| err.to_string())?;
        out.extend(unit.to_vec2::<f32>().map_err(|err| err.to_string())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_follow_each_models_format() {
        let msgs = vec![json!({"role": "system", "content": "Be brief."}), json!({"role": "user", "content": [{"type": "text", "text": "Hi"}]}), json!({"role": "assistant", "content": "Hello!"}), json!({"role": "tool", "content": "42"}), json!({"role": "user", "content": "Thanks"})];
        let t = turns(&msgs);
        assert_eq!(t[1], ("user".into(), "Hi".into()));
        assert_eq!(t[3], ("user".into(), "Tool result:\n42".into()));
        let chatml = render(Template::ChatMl, &t);
        assert!(chatml.starts_with("<|im_start|>system\nBe brief.<|im_end|>\n<|im_start|>user\nHi<|im_end|>"));
        assert!(chatml.ends_with("<|im_start|>assistant\n"));
        let llama = render(Template::Llama3, &t);
        assert!(llama.starts_with("<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\nBe brief.<|eot_id|>"));
        assert!(llama.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
    }

    #[test]
    fn mel_filters_match_whispers() {
        let f = mel_filters(80);
        assert_eq!(f.len(), 80 * 201);
        // Each filter is a triangle: non-negative, with weight somewhere, and the first rises from 0 Hz.
        assert!(f.iter().all(|v| *v >= 0.0));
        assert!((0..80).all(|m| f[m * 201..(m + 1) * 201].iter().any(|v| *v > 0.0)));
        assert_eq!(f[0], 0.0);
        assert!(f[1] > 0.0);
    }

    #[test]
    fn wav_files_become_16khz_mono() {
        // 0.1 s of 8 kHz stereo 16-bit: left at half scale, right silent.
        let frames = 800u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + frames * 4).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&(8000u32 * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(frames * 4).to_le_bytes());
        for _ in 0..frames {
            wav.extend_from_slice(&16384i16.to_le_bytes());
            wav.extend_from_slice(&0i16.to_le_bytes());
        }
        let s = wav_samples(&wav).unwrap();
        assert_eq!(s.len(), 1600);
        assert!((s[100] - 0.25).abs() < 1e-3);
        assert!(wav_samples(b"not audio").is_err());
    }

    /// With a model downloaded (`LUMO_TEST_MODELS=<local data dir>`): a real answer and its speed.
    #[test]
    #[ignore]
    fn a_downloaded_model_answers() {
        let Ok(dir) = std::env::var("LUMO_TEST_MODELS") else { return };
        let root = Path::new(&dir).join("models");
        let long_system = "You are a helpful assistant. ".repeat(std::env::var("LUMO_TEST_SYSTEM_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(1));
        let msgs = vec![json!({"role": "system", "content": long_system}), json!({"role": "user", "content": "What is the capital of Portugal? One sentence."})];
        // Loaded once first, so the timing is of answering.
        generate(&root, "qwen2.5-0.5b", &msgs[1..], &Params { temperature: Some(0.0), top_p: None, max_tokens: Some(1) }, |_| true).unwrap();
        let started = std::time::Instant::now();
        let mut first: Option<std::time::Duration> = None;
        let mut text = String::new();
        let usage = generate(&root, "qwen2.5-0.5b", &msgs, &Params { temperature: Some(0.0), top_p: None, max_tokens: Some(40) }, |p| {
            first.get_or_insert(started.elapsed());
            text.push_str(p);
            true
        })
        .unwrap();
        eprintln!("prompt {} tokens, answer {} tokens, first piece after {:?}, all in {:?}: {text}", usage.prompt, usage.completion, first, started.elapsed());
        assert!(text.contains("Lisbon"), "{text}");
    }

    /// With Whisper downloaded (`LUMO_TEST_MODELS`) and a recording (`LUMO_TEST_WAV`).
    #[test]
    #[ignore]
    fn a_downloaded_whisper_transcribes() {
        let (Ok(dir), Ok(wav)) = (std::env::var("LUMO_TEST_MODELS"), std::env::var("LUMO_TEST_WAV")) else { return };
        let root = Path::new(&dir).join("models");
        let samples = audio_samples(&std::fs::read(wav).unwrap()).unwrap();
        let started = std::time::Instant::now();
        let text = transcribe(&root, "whisper-tiny", &samples, "").unwrap();
        eprintln!("{:.1} s of audio in {:?}: {text}", samples.len() as f64 / 16_000.0, started.elapsed());
        assert!(text.to_lowercase().contains("country"), "{text}");
    }

    #[test]
    fn the_queue_has_a_limit_and_frees_places() {
        let held: Vec<Place> = (0..QUEUE_LIMIT).map_while(|_| Place::take()).collect();
        assert!(held.len() <= QUEUE_LIMIT);
        if held.len() == QUEUE_LIMIT {
            assert!(Place::take().is_none(), "full");
        }
        drop(held);
        assert!(Place::take().is_some());
    }

    /// With MiniLM downloaded (`LUMO_TEST_MODELS`): similar sentences land close together.
    #[test]
    #[ignore]
    fn a_downloaded_embedder_groups_meanings() {
        let Ok(dir) = std::env::var("LUMO_TEST_MODELS") else { return };
        let root = Path::new(&dir).join("models");
        let v = embed_texts(&root, "minilm-l6", &["How do I cancel the contract?".into(), "Termination clause: either party may end the agreement with 30 days notice.".into(), "The bakery sells sourdough on Saturdays.".into()]).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].len(), 384);
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        eprintln!("contract~termination {:.3}, contract~bakery {:.3}", dot(&v[0], &v[1]), dot(&v[0], &v[2]));
        assert!(dot(&v[0], &v[1]) > dot(&v[0], &v[2]) + 0.1);
    }

    #[test]
    fn the_catalog_is_well_formed() {
        for e in CATALOG {
            assert!(e.files.iter().any(|f| f.2 == "tokenizer.json"), "{}", e.id);
            match e.kind {
                Kind::Chat => assert!(e.files.iter().any(|f| f.2 == "model.gguf")),
                Kind::Speech | Kind::Embedding => assert!(e.files.iter().any(|f| f.2 == "model.safetensors") && e.files.iter().any(|f| f.2 == "config.json")),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(list(dir.path()).iter().all(|v| !v.installed));
        assert!(models(dir.path()).is_empty());
    }
}
