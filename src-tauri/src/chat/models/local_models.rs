//! The built-in model runtime (Osaurus's and LM Studio's local models, OpenBot's on-device
//! dictation): chat models and Whisper speech-to-text run inside Cortex, on this computer's CPU,
//! with Hugging Face's `candle` (pure Rust). Nothing to install: a model is downloaded once from
//! Hugging Face (Settings → Providers → Built-in models) and works offline after that.
//!
//! - Chat models (GGUF, 4-bit): Qwen 2.5 and Llama 3.2. They appear in the model picker as
//!   `local/<id>`. The engine reaches them through a small OpenAI-compatible endpoint on
//!   127.0.0.1, started on first use, so streaming and every other path work as for any provider.
//! - Speech: Whisper (tiny to large-v3-turbo) as the "Built-in" speech-to-text engine, for the
//!   composer's microphone and for dictation anywhere.

use crate::chat::error::ChatError;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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
    Entry {
        id: "qwen2.5-0.5b",
        name: "Qwen 2.5 0.5B",
        kind: Kind::Chat,
        size_mb: 500,
        note: "Tiny and quick: short answers, rewrites, titles. Any computer.",
        files: &[
            (
                "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
                "qwen2.5-0.5b-instruct-q4_k_m.gguf",
                "model.gguf",
            ),
            (
                "Qwen/Qwen2.5-0.5B-Instruct",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Entry {
        id: "qwen2.5-1.5b",
        name: "Qwen 2.5 1.5B",
        kind: Kind::Chat,
        size_mb: 1_100,
        note: "Good everyday answers on most laptops.",
        files: &[
            (
                "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
                "qwen2.5-1.5b-instruct-q4_k_m.gguf",
                "model.gguf",
            ),
            (
                "Qwen/Qwen2.5-1.5B-Instruct",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Entry {
        id: "qwen2.5-3b",
        name: "Qwen 2.5 3B",
        kind: Kind::Chat,
        size_mb: 2_000,
        note: "Better reasoning and writing; 8 GB of memory or more.",
        files: &[
            (
                "Qwen/Qwen2.5-3B-Instruct-GGUF",
                "qwen2.5-3b-instruct-q4_k_m.gguf",
                "model.gguf",
            ),
            (
                "Qwen/Qwen2.5-3B-Instruct",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Entry {
        id: "qwen2.5-7b",
        name: "Qwen 2.5 7B",
        kind: Kind::Chat,
        size_mb: 4_700,
        note: "The strongest here; 16 GB of memory, slower on a laptop.",
        files: &[
            (
                "bartowski/Qwen2.5-7B-Instruct-GGUF",
                "Qwen2.5-7B-Instruct-Q4_K_M.gguf",
                "model.gguf",
            ),
            (
                "Qwen/Qwen2.5-7B-Instruct",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Entry {
        id: "llama-3.2-1b",
        name: "Llama 3.2 1B",
        kind: Kind::Chat,
        size_mb: 810,
        note: "Meta's small model; quick, English first.",
        files: &[
            (
                "bartowski/Llama-3.2-1B-Instruct-GGUF",
                "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
                "model.gguf",
            ),
            (
                "unsloth/Llama-3.2-1B-Instruct",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Entry {
        id: "llama-3.2-3b",
        name: "Llama 3.2 3B",
        kind: Kind::Chat,
        size_mb: 2_000,
        note: "Meta's mid-size model; 8 GB of memory or more.",
        files: &[
            (
                "bartowski/Llama-3.2-3B-Instruct-GGUF",
                "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
                "model.gguf",
            ),
            (
                "unsloth/Llama-3.2-3B-Instruct",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Entry {
        id: "minilm-l6",
        name: "MiniLM L6 (embeddings)",
        kind: Kind::Embedding,
        size_mb: 90,
        note: "Semantic search for your libraries, on this computer (English first).",
        files: &[
            (
                "sentence-transformers/all-MiniLM-L6-v2",
                "model.safetensors",
                "model.safetensors",
            ),
            (
                "sentence-transformers/all-MiniLM-L6-v2",
                "config.json",
                "config.json",
            ),
            (
                "sentence-transformers/all-MiniLM-L6-v2",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Entry {
        id: "whisper-tiny",
        name: "Whisper tiny",
        kind: Kind::Speech,
        size_mb: 150,
        note: "Fast dictation; fine for clear speech.",
        files: &[
            (
                "openai/whisper-tiny",
                "model.safetensors",
                "model.safetensors",
            ),
            ("openai/whisper-tiny", "config.json", "config.json"),
            ("openai/whisper-tiny", "tokenizer.json", "tokenizer.json"),
        ],
    },
    Entry {
        id: "whisper-base",
        name: "Whisper base",
        kind: Kind::Speech,
        size_mb: 290,
        note: "A good balance for dictation.",
        files: &[
            (
                "openai/whisper-base",
                "model.safetensors",
                "model.safetensors",
            ),
            ("openai/whisper-base", "config.json", "config.json"),
            ("openai/whisper-base", "tokenizer.json", "tokenizer.json"),
        ],
    },
    Entry {
        id: "whisper-small",
        name: "Whisper small",
        kind: Kind::Speech,
        size_mb: 970,
        note: "More accurate, other languages too; slower.",
        files: &[
            (
                "openai/whisper-small",
                "model.safetensors",
                "model.safetensors",
            ),
            ("openai/whisper-small", "config.json", "config.json"),
            ("openai/whisper-small", "tokenizer.json", "tokenizer.json"),
        ],
    },
    Entry {
        id: "whisper-large-v3-turbo",
        name: "Whisper large-v3 turbo",
        kind: Kind::Speech,
        size_mb: 1_620,
        note: "The most accurate; best with a fast computer.",
        files: &[
            (
                "openai/whisper-large-v3-turbo",
                "model.safetensors",
                "model.safetensors",
            ),
            (
                "openai/whisper-large-v3-turbo",
                "config.json",
                "config.json",
            ),
            (
                "openai/whisper-large-v3-turbo",
                "tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
];

pub fn entry(id: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.id == id)
}

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// What the built-in models have running: downloads, the loaded weights, the requests waiting
/// for them, and the embedder and speech model once loaded.
#[derive(Default)]
struct LocalModels {
    /// Models being downloaded, and those asked to stop.
    downloads: Mutex<Vec<String>>,
    cancelled: Mutex<Vec<String>>,
    embedder: Mutex<Option<embedding::Embedder>>,
    /// The chat model in memory; fair, so requests take turns in arrival order.
    loaded: parking_lot::FairMutex<Option<generation::Loaded>>,
    /// Requests waiting for the runtime's thread, and the signal that one arrived.
    pending: (
        Mutex<std::collections::VecDeque<generation::Job>>,
        std::sync::Condvar,
    ),
    /// Requests waiting for or using the processor.
    queued: std::sync::atomic::AtomicUsize,
    speech: Mutex<Option<speech::Speech>>,
}

// Process-wide: a model's weights take gigabytes and the processor runs one at a time, so there is
// one of each per process. The model layer below the turn (connecting a model, embedding a text,
// transcribing) is reached with no app state, from the app, the CLI and the local API alike.
static LOCAL_MODELS: std::sync::LazyLock<LocalModels> =
    std::sync::LazyLock::new(LocalModels::default);

/// Where models are kept: the app's local data folder (not the settings folder, so backups and
/// sync leave gigabytes of weights alone). Set once at start.
pub(crate) fn init(local_data_dir: &Path) {
    let _ = ROOT.set(local_data_dir.join("models"));
}

pub fn root() -> Result<PathBuf, ChatError> {
    ROOT.get()
        .cloned()
        .ok_or_else(|| ChatError::Invalid("The model folder is not set up yet.".into()))
}

fn model_dir(root: &Path, id: &str) -> Result<PathBuf, ChatError> {
    entry(id)
        .ok_or_else(|| format!("Unknown model `{id}`."))
        .map_err(ChatError::Invalid)?;
    Ok(root.join(id))
}

pub(crate) fn installed(root: &Path, id: &str) -> bool {
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
        .map(|e| View {
            id: e.id.into(),
            name: e.name.into(),
            kind: e.kind,
            size_mb: e.size_mb,
            note: e.note.into(),
            installed: installed(root, e.id),
            downloading: downloads()
                .lock()
                .is_ok_and(|d| d.contains(&e.id.to_string())),
        })
        .collect()
}

/// The chat models downloaded, for the model picker.
pub(in crate::chat) fn models(root: &Path) -> Vec<crate::chat::providers::ModelView> {
    CATALOG
        .iter()
        .filter(|e| e.kind == Kind::Chat && installed(root, e.id))
        .map(|e| crate::chat::providers::ModelView {
            reference: format!("{VENDOR}/{}", e.id),
            vendor: VENDOR.into(),
            provider_label: "Built-in (on this computer)".into(),
            id: e.id.into(),
            name: e.name.into(),
            reasoning: false,
            tools: false,
            context_window: Some(CONTEXT as u32),
        })
        .collect()
}

mod downloads;
mod embedding;
mod generation;
mod server;
mod speech;
pub(crate) use downloads::*;
pub(in crate::chat) use embedding::*;
pub use generation::*;
pub use server::*;
pub(in crate::chat) use speech::*;

#[cfg(test)]
mod tests;
