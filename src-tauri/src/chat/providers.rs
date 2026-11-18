//! Model providers: which ones the user configured, their keys and their models.
//!
//! The list of providers is curated here for display; rig's registry decides whether each is
//! reachable in this build ([`ProviderId::catalog`]). Keys live in the OS keychain under the
//! local account, never in the settings file, which only records that one is saved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rig_core::catalog::{Catalog, ReasoningSupport};
use rig_core::operation::Completion;
use rig_core::providers::registry::{ConnectOptions, ProviderId};
use rig_core::DynModel;
use serde::{Deserialize, Serialize};

/// How a provider is shown, and where its key comes from.
pub struct ProviderInfo {
    /// rig's vendor name.
    pub vendor: &'static str,
    pub label: &'static str,
    /// Where the user gets a key; empty for local servers.
    pub key_url: &'static str,
    /// Runs on the user's machine.
    pub local: bool,
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo { vendor: "openai", label: "OpenAI", key_url: "https://platform.openai.com/api-keys", local: false },
    ProviderInfo { vendor: "anthropic", label: "Anthropic", key_url: "https://console.anthropic.com/settings/keys", local: false },
    ProviderInfo { vendor: "gcp.gemini", label: "Google Gemini", key_url: "https://aistudio.google.com/apikey", local: false },
    ProviderInfo { vendor: "mistral", label: "Mistral", key_url: "https://console.mistral.ai/api-keys", local: false },
    ProviderInfo { vendor: "xai", label: "xAI (Grok)", key_url: "https://console.x.ai", local: false },
    ProviderInfo { vendor: "deepseek", label: "DeepSeek", key_url: "https://platform.deepseek.com/api_keys", local: false },
    ProviderInfo { vendor: "openrouter", label: "OpenRouter", key_url: "https://openrouter.ai/keys", local: false },
    ProviderInfo { vendor: "groq", label: "Groq", key_url: "https://console.groq.com/keys", local: false },
    ProviderInfo { vendor: "together", label: "Together AI", key_url: "https://api.together.ai/settings/api-keys", local: false },
    ProviderInfo { vendor: "perplexity", label: "Perplexity", key_url: "https://www.perplexity.ai/settings/api", local: false },
    ProviderInfo { vendor: "cohere", label: "Cohere", key_url: "https://dashboard.cohere.com/api-keys", local: false },
    ProviderInfo { vendor: "moonshot", label: "Moonshot (Kimi)", key_url: "https://platform.moonshot.ai/console/api-keys", local: false },
    ProviderInfo { vendor: "zai", label: "Z.ai (GLM)", key_url: "https://z.ai/manage-apikey/apikey-list", local: false },
    ProviderInfo { vendor: "ollama", label: "Ollama", key_url: "", local: true },
    ProviderInfo { vendor: "llamacpp", label: "llama.cpp, LM Studio or vLLM", key_url: "", local: true },
];

pub fn info(vendor: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.vendor == vendor)
}

/// Keychain entry name for a vendor's key.
pub fn secret_name(vendor: &str) -> String {
    format!("llm-provider:{vendor}")
}

/// One provider's saved settings. The key itself is in the keychain.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub has_key: bool,
    pub base_url: Option<String>,
    /// Model ids the user added by hand, for models the catalog does not list.
    pub custom_models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProvidersFile {
    pub providers: BTreeMap<String, ProviderSettings>,
    /// `vendor/model` the composer starts on.
    pub default_model: Option<String>,
    /// `vendor/model` for `generate_image`; a default for the providers set up when unset.
    pub image_model: Option<String>,
    /// `vendor/model` for voice input; a default for the providers set up when unset.
    pub speech_model: Option<String>,
    /// `vendor/model` for reading answers aloud; a default for the providers set up when unset.
    pub voice_model: Option<String>,
    /// `vendor/model` a reply switches to, once, when its model fails before answering (a provider
    /// down, an account out of credit); an agent's own backup comes first.
    #[serde(default)]
    pub backup_model: Option<String>,
}

pub fn file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("chat").join("providers.json")
}

pub fn load(config_dir: &Path) -> ProvidersFile {
    std::fs::read_to_string(file_path(config_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, file: &ProvidersFile) -> Result<(), String> {
    super::store::write_json(&file_path(config_dir), file)
}

/// What the settings page shows for a provider.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub vendor: String,
    pub label: String,
    pub key_url: String,
    pub local: bool,
    pub requires_key: bool,
    pub default_base_url: String,
    /// The environment variable a key is read from when none is saved.
    pub env_key: Option<String>,
    pub settings: ProviderSettings,
    /// Configured well enough to offer its models.
    pub ready: bool,
}

/// The endpoint rig sends to when no base URL is set.
pub fn default_base_url(vendor: &str) -> Option<String> {
    let config = ProviderId::catalog(vendor)?.config(String::new())?;
    Some(config.base_url().to_string())
}

/// The user's base URL as rig expects it. Some dialects put `/v1` in their request paths and keep
/// it out of the base URL; a user pasting `https://gateway/v1` for one of those would otherwise
/// send to `/v1/v1/...`.
pub fn normalize_base_url(vendor: &str, base_url: &str) -> String {
    let url = base_url.trim().trim_end_matches('/');
    let default_has_v1 = default_base_url(vendor)
        .is_none_or(|d| d.trim_end_matches('/').ends_with("/v1"));
    match url.strip_suffix("/v1") {
        Some(root) if !default_has_v1 => root.to_string(),
        _ => url.to_string(),
    }
}

pub fn requires_key(vendor: &str) -> bool {
    ProviderId::catalog(vendor).is_some_and(|id| id.requires_credential())
}

/// The environment variable holding a key for `vendor`, if one is set, such as
/// `OPENAI_API_KEY`. rig reads it when no saved key is given, which also covers machines without
/// a keychain.
pub fn env_key(vendor: &str) -> Option<&'static str> {
    ProviderId::catalog(vendor)?
        .api_key_envs()
        .into_iter()
        .find(|name| std::env::var(name).is_ok_and(|v| !v.trim().is_empty()))
}

pub fn is_ready(vendor: &str, settings: &ProviderSettings) -> bool {
    settings.enabled && (settings.has_key || !requires_key(vendor) || env_key(vendor).is_some())
}

pub fn views(file: &ProvidersFile) -> Vec<ProviderView> {
    PROVIDERS
        .iter()
        .filter(|p| ProviderId::catalog(p.vendor).is_some())
        .map(|p| {
            let settings = file.providers.get(p.vendor).cloned().unwrap_or_default();
            ProviderView {
                vendor: p.vendor.into(),
                label: p.label.into(),
                key_url: p.key_url.into(),
                local: p.local,
                requires_key: requires_key(p.vendor),
                default_base_url: default_base_url(p.vendor).unwrap_or_default(),
                env_key: env_key(p.vendor).map(Into::into),
                ready: is_ready(p.vendor, &settings),
                settings,
            }
        })
        .collect()
}

/// A model the composer can pick.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    /// `vendor/model`.
    pub reference: String,
    pub vendor: String,
    pub provider_label: String,
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    pub tools: bool,
    pub context_window: Option<u32>,
}

/// Models the catalog lists for `vendor` that can hold a conversation and call tools.
fn catalog_models(vendor: &str) -> Vec<ModelView> {
    let Some(provider) = ProviderId::catalog(vendor) else {
        return Vec::new();
    };
    let label = info(vendor).map_or(vendor, |p| p.label);
    let mut models: Vec<ModelView> = Catalog::builtin()
        .iter()
        .filter(|spec| spec.provider.vendor() == provider.vendor())
        .filter(|spec| !spec.deprecated && spec.input.text && spec.tools)
        .map(|spec| ModelView {
            reference: format!("{vendor}/{}", spec.id),
            vendor: vendor.into(),
            provider_label: label.into(),
            id: spec.id.clone(),
            name: spec.display_name.clone(),
            reasoning: !matches!(spec.reasoning, ReasoningSupport::None),
            tools: spec.tools,
            context_window: spec.context_window,
        })
        .collect();
    models.sort_by(|a, b| a.name.cmp(&b.name));
    models.dedup_by(|a, b| a.id == b.id);
    models
}

fn custom_model(vendor: &str, id: &str) -> ModelView {
    let label = info(vendor).map_or(vendor, |p| p.label);
    let spec = Catalog::builtin()
        .resolve(&format!("{vendor}/{id}"))
        .ok()
        .map(|r| r.spec.clone());
    ModelView {
        reference: format!("{vendor}/{id}"),
        vendor: vendor.into(),
        provider_label: label.into(),
        id: id.into(),
        name: spec.as_ref().map_or_else(|| id.to_string(), |s| s.display_name.clone()),
        reasoning: spec
            .as_ref()
            .is_some_and(|s| !matches!(s.reasoning, ReasoningSupport::None)),
        tools: spec.as_ref().is_none_or(|s| s.tools),
        context_window: spec.and_then(|s| s.context_window),
    }
}

/// Models served by a running Ollama daemon, from its own listing endpoint.
async fn ollama_models(base_url: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Tags {
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }
    let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build();
    let Ok(client) = client else { return Vec::new() };
    match client.get(format!("{root}/api/tags")).send().await {
        Ok(response) => response
            .json::<Tags>()
            .await
            .map(|tags| tags.models.into_iter().map(|t| t.name).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Model ids an OpenAI-compatible server lists at `/v1/models` (llama.cpp's server, LM Studio,
/// vLLM, LocalAI, Jan): the models actually loaded there.
async fn openai_compatible_models(base_url: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct List {
        data: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        id: String,
    }
    let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(2)).build() else { return Vec::new() };
    match client.get(format!("{root}/v1/models")).send().await {
        Ok(response) => response.json::<List>().await.map(|l| l.data.into_iter().map(|e| e.id).filter(|id| !id.is_empty()).collect()).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Every model of every ready provider. A local server's own listing replaces the catalog's,
/// which names models the user may not have pulled.
pub async fn models(file: &ProvidersFile) -> Vec<ModelView> {
    let mut out = Vec::new();
    for provider in PROVIDERS {
        let Some(settings) = file.providers.get(provider.vendor) else {
            continue;
        };
        if !is_ready(provider.vendor, settings) {
            continue;
        }
        let mut models = if provider.vendor == "ollama" {
            let base = settings
                .base_url
                .clone()
                .or_else(|| default_base_url(provider.vendor))
                .unwrap_or_default();
            ollama_models(&base)
                .await
                .iter()
                .map(|id| custom_model(provider.vendor, id))
                .collect()
        } else if provider.local {
            let base = settings.base_url.clone().or_else(|| default_base_url(provider.vendor)).unwrap_or_default();
            openai_compatible_models(&base).await.iter().map(|id| custom_model(provider.vendor, id)).collect()
        } else {
            catalog_models(provider.vendor)
        };
        for id in &settings.custom_models {
            if !models.iter().any(|m| &m.id == id) {
                models.push(custom_model(provider.vendor, id));
            }
        }
        out.extend(models);
    }
    out
}

/// Splits `vendor/model` at the first `/`; model ids can hold more (`openrouter/a/b`).
pub fn split_reference(reference: &str) -> Option<(&str, &str)> {
    reference
        .split_once('/')
        .filter(|(vendor, model)| !vendor.is_empty() && !model.is_empty())
}

/// The model `reference` names, connected with the user's key and base URL.
pub fn connect(
    reference: &str,
    settings: &ProviderSettings,
    api_key: Option<String>,
) -> Result<DynModel<Completion>, String> {
    let (vendor, _) =
        split_reference(reference).ok_or_else(|| format!("`{reference}` is not vendor/model"))?;
    let mut options = ConnectOptions::new();
    match api_key.filter(|k| !k.is_empty()) {
        Some(key) => options = options.api_key(key),
        // rig reads the variable itself.
        None if env_key(vendor).is_some() => {}
        None if requires_key(vendor) => {
            return Err(format!(
                "Add an API key for {} in Settings → Providers.",
                info(vendor).map_or(vendor, |p| p.label)
            ))
        }
        // rig reads the key from the environment when none is given; a local server needs none,
        // and an empty one keeps it from picking up an unrelated variable.
        None => options = options.api_key(String::new()),
    }
    if let Some(base_url) = settings.base_url.as_ref().filter(|u| !u.trim().is_empty()) {
        options = options.base_url(normalize_base_url(vendor, base_url));
    }
    Catalog::builtin()
        .connect_with(reference, options)
        .map_err(|e| e.to_string())
}

/// Whether the catalog says `reference` can reason, so a "think" request is not refused.
pub fn supports_reasoning(reference: &str) -> bool {
    Catalog::builtin()
        .resolve(reference)
        .is_ok_and(|r| !matches!(r.spec.reasoning, ReasoningSupport::None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_openai_compatible_servers_list_their_models() {
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get(|| async { axum::Json(serde_json::json!({ "object": "list", "data": [{ "id": "qwen3-8b-q4" }, { "id": "llama-3.2-3b" }] })) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        // With or without /v1 in the address the user typed.
        assert_eq!(openai_compatible_models(&format!("http://{addr}/v1")).await, ["qwen3-8b-q4", "llama-3.2-3b"]);
        assert_eq!(openai_compatible_models(&format!("http://{addr}")).await.len(), 2);
        assert!(openai_compatible_models("http://127.0.0.1:9").await.is_empty(), "a server that is off lists nothing");
    }

    #[test]
    fn curated_providers_are_known_to_rig() {
        for provider in PROVIDERS {
            assert!(
                ProviderId::catalog(provider.vendor).is_some(),
                "rig does not know {}",
                provider.vendor
            );
        }
    }

    #[test]
    fn cloud_providers_need_keys_and_local_ones_do_not() {
        assert!(requires_key("openai"));
        assert!(requires_key("anthropic"));
        assert!(!requires_key("ollama"));
    }

    #[test]
    fn catalog_lists_chat_models() {
        let models = catalog_models("openai");
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.reference.starts_with("openai/") && m.tools));
    }

    #[test]
    fn base_urls_follow_the_dialect() {
        // xAI's paths carry /v1, so a pasted /v1 is dropped; OpenAI's base keeps it.
        assert_eq!(normalize_base_url("xai", "https://gw.example/v1/"), "https://gw.example");
        assert_eq!(normalize_base_url("openai", "https://gw.example/v1"), "https://gw.example/v1");
        assert_eq!(normalize_base_url("ollama", "http://box:11434/v1"), "http://box:11434/v1");
        assert_eq!(default_base_url("ollama").as_deref(), Some("http://localhost:11434/v1"));
    }

    #[test]
    fn references_split_at_the_first_slash() {
        assert_eq!(
            split_reference("openrouter/meta/llama"),
            Some(("openrouter", "meta/llama"))
        );
        assert_eq!(split_reference("openai"), None);
        assert_eq!(split_reference("/x"), None);
    }

    #[test]
    fn a_missing_key_is_reported_before_connecting() {
        if env_key("openai").is_some() {
            return; // a developer's own key would satisfy it
        }
        let error = connect("openai/gpt-5.5", &ProviderSettings::default(), None).unwrap_err();
        assert!(error.contains("OpenAI"), "{error}");
    }

    #[test]
    fn a_keyed_provider_connects_without_network() {
        let model = connect(
            "openai/gpt-5.5",
            &ProviderSettings::default(),
            Some("sk-test".into()),
        );
        assert!(model.is_ok(), "{:?}", model.err());
    }

    #[test]
    fn readiness_needs_enabled_and_a_key_when_required() {
        let mut settings = ProviderSettings { enabled: true, ..Default::default() };
        assert_eq!(is_ready("openai", &settings), env_key("openai").is_some());
        assert!(is_ready("ollama", &settings));
        settings.has_key = true;
        assert!(is_ready("openai", &settings));
        settings.enabled = false;
        assert!(!is_ready("openai", &settings));
    }
}

