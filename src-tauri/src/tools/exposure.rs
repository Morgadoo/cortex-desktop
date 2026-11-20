//! Keeps prompt injection from carrying private data out through tool calls.
//!
//! An injected instruction can only exfiltrate once the model has seen both private data and
//! untrusted text, and can then call a tool that sends data somewhere. Reading mail supplies both
//! at once, and so does reading from a connected service such as Slack, Jira or GitHub, whose
//! messages and tickets outsiders write. From then on, tools that can send data out need the user's approval however they are
//! configured, and in a scheduled run, where nobody can approve, they are refused.
//!
//! The descriptions of tools from MCP servers the user added by hand count as untrusted text
//! too, for as long as such a server is offered to the model: they reach the model with every
//! request, written by whoever runs the server. So do the server's own hints, which is why its
//! tools are treated as able to send data whatever they claim.
//!
//! Hidden characters in what an outbound tool would send (see [`hidden_text`]) need approval
//! whatever the conversation has read, since they can carry data the user can't see.
//!
//! Exposure is tracked per conversation when the web app names it, and kept across restarts so a
//! resumed conversation is still covered. Scheduled runs are tracked per run. Calls that name
//! neither fall back to the account session, which every call contributes to.
//!
//! Inspecting the request can't be the boundary: data fits in a path, a host name or an
//! encoding. The one exemption is a plain GET of a URL that appeared verbatim in an earlier mail
//! or web result of the same conversation, which carries nothing the attacker did not already
//! write. Results of other tools can echo what the model sent them, so their URLs don't count.
//! Which links are followed can still encode a little, so the exemption is capped per
//! conversation.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::hidden_text;
use super::registry::ToolDefinition;

const MAX_SEEN_URLS: usize = 10_000;
const MAX_TRACKED_RUNS: usize = 64;
/// Persisted, so bounded; the oldest conversations are forgotten first.
pub(crate) const MAX_TRACKED_CONVERSATIONS: usize = 5_000;
const MAX_UNPROMPTED_LINKS: usize = 20;

const REFUSED_IN_RUN: &str = "this scheduled run has read email or other untrusted content, so \
     tools that can send data out or change files are disabled for the rest of the run. To let \
     scheduled tasks use this tool anyway, set it to Always allow in Cortex's connector settings";
const REFUSED_HIDDEN: &str = "the request contains invisible characters, which a scheduled run \
     cannot send without someone approving them";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exposure {
    #[serde(default)]
    private: bool,
    #[serde(default)]
    mail: bool,
    #[serde(default)]
    web: bool,
    #[serde(default)]
    shell: bool,
    /// Results from MCP servers and from connectors this module doesn't know.
    #[serde(default)]
    service: bool,
    /// Descriptions of tools from a server added by hand. Never recorded, only added at the gate.
    #[serde(default, skip_serializing)]
    tools: bool,
    /// Order of first exposure, so the oldest conversations are evicted first.
    #[serde(default)]
    pub(crate) seq: u64,
}

impl Exposure {
    /// Merges `other`'s flags, returning whether anything changed.
    pub(crate) fn merge(&mut self, other: Exposure) -> bool {
        let before = *self;
        self.private |= other.private;
        self.mail |= other.mail;
        self.web |= other.web;
        self.shell |= other.shell;
        self.service |= other.service;
        before.flags() != self.flags()
    }

    fn flags(self) -> [bool; 5] {
        [self.private, self.mail, self.web, self.shell, self.service]
    }

    pub(crate) fn is_empty(self) -> bool {
        self.flags() == [false; 5]
    }

    /// Adds the descriptions of hand-added servers' tools, when one is offered to the model.
    pub(crate) fn with_unvetted_tools(self, offered: bool) -> Self {
        Exposure {
            tools: offered,
            ..self
        }
    }

    fn is_untrusted(self) -> bool {
        self.mail || self.web || self.shell || self.service || self.tools
    }

    fn is_risky(self) -> bool {
        self.private && self.is_untrusted()
    }

    fn sources(self) -> Vec<&'static str> {
        [
            (self.mail, "mail"),
            (self.web, "web"),
            (self.shell, "shell"),
            (self.service, "service"),
            (self.tools, "connector"),
        ]
        .into_iter()
        .filter_map(|(seen, name)| seen.then_some(name))
        .collect()
    }
}

/// What a call's result adds to the model's context. Mail, web pages, shell output and what MCP
/// servers return can be written by anyone; local files and Luminance are the user's own. A
/// connector this function doesn't name counts as an outside service, so a new one is gated
/// until someone classifies it.
fn exposure_of(connector_id: &str, tool_name: &str) -> Exposure {
    let private = Exposure {
        private: true,
        ..Default::default()
    };
    match (connector_id, tool_name) {
        ("proton_mail", "list_messages" | "search_messages" | "read_message") => Exposure {
            mail: true,
            ..private
        },
        ("browser", _) => Exposure {
            web: true,
            ..Default::default()
        },
        ("bash", _) => Exposure {
            shell: true,
            ..private
        },
        ("proton_mail", _) | ("filesystem" | "luminance", _) => private,
        ("echo", _) => Exposure::default(),
        _ => Exposure {
            service: true,
            ..private
        },
    }
}

/// Whether a tool can send data off the machine. Luminance writes go to Proton but can share
/// documents onwards. An MCP tool is trusted only when a vetted server marks it read-only and
/// closed to the open world; the spec reads an absent `openWorldHint` as open. A tool this
/// function doesn't name is assumed to send, so a new one is gated until someone classifies it.
fn sends_data(
    connector_id: &str,
    tool_name: &str,
    tool: Option<&ToolDefinition>,
    vetted: bool,
) -> bool {
    let read_only = tool.and_then(|tool| tool.read_only) == Some(true);
    let open_world = tool.and_then(|tool| tool.open_world) != Some(false);
    match (connector_id, tool_name) {
        ("proton_mail", "list_messages" | "search_messages" | "read_message") => false,
        ("filesystem" | "echo", _) => false,
        ("luminance", _) => !read_only,
        // A screenshot sends nothing; a click or keystroke can submit anything on screen.
        ("computer", _) => !read_only,
        // A tool the assistant wrote reaches the network only when the user granted it hosts.
        ("script", _) => !read_only,
        (id, _) if id.starts_with("mcp-") => !vetted || !read_only || open_world,
        _ => true,
    }
}

/// Whether a tool changes the user's files. Untrusted content alone is enough to gate it: a
/// planted script or a rewritten config acts later, outside any approval.
fn changes_files(connector_id: &str, tool: Option<&ToolDefinition>) -> bool {
    connector_id == "filesystem" && tool.and_then(|tool| tool.read_only) != Some(true)
}

/// The URL of a browser call that sends nothing but the URL itself.
fn plain_get_url(connector_id: &str, tool_name: &str, input: &Value) -> Option<String> {
    let is_empty = |key: &str| match input.get(key) {
        None | Some(Value::Null) => true,
        Some(Value::Object(map)) => map.is_empty(),
        Some(_) => false,
    };
    let plain = match (connector_id, tool_name) {
        ("browser", "navigate") => true,
        ("browser", "fetch") => {
            input
                .get("method")
                .and_then(Value::as_str)
                .is_none_or(|method| method.eq_ignore_ascii_case("GET"))
                && is_empty("headers")
                && is_empty("body")
        }
        _ => false,
    };
    plain
        .then(|| input.get("url").and_then(Value::as_str))
        .flatten()
        .and_then(normalize)
}

fn destination(connector_id: &str, input: &Value) -> Option<String> {
    (connector_id == "browser")
        .then(|| input.get("url").and_then(Value::as_str))
        .flatten()
        .and_then(|url| url::Url::parse(url).ok())
        .and_then(|url| url.host_str().map(str::to_owned))
}

fn normalize(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}

fn collect_urls(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => urls_in(text, out),
        Value::Array(items) => items.iter().for_each(|item| collect_urls(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_urls(item, out)),
        _ => {}
    }
}

fn urls_in(text: &str, out: &mut Vec<String>) {
    let mut rest = text;
    while let Some(start) = rest.find("http") {
        let candidate = &rest[start..];
        let end = candidate
            .find(|c: char| c.is_whitespace() || "\"'<>`\\{}|^()[]".contains(c))
            .unwrap_or(candidate.len());
        let raw = candidate[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if raw.starts_with("http://") || raw.starts_with("https://") {
            if let Some(url) = normalize(&raw.replace("&amp;", "&")) {
                out.push(url);
            }
        }
        rest = &candidate[end.max(4)..];
    }
}

/// Which exposure a call belongs to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Scope<'a> {
    Run(&'a str),
    Conversation(&'a str),
    Session,
}

impl<'a> Scope<'a> {
    /// A conversation id the web app supplied, if it is one; anything else falls back to the
    /// session, which every call contributes to.
    pub(crate) fn of(run: Option<&'a str>, conversation: Option<&'a str>) -> Self {
        let valid = |id: &&str| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        };
        match (run, conversation.filter(valid)) {
            (Some(run), _) => Scope::Run(run),
            (None, Some(conversation)) => Scope::Conversation(conversation),
            (None, None) => Scope::Session,
        }
    }

    fn key(self) -> String {
        match self {
            Scope::Run(id) => format!("run:{id}"),
            Scope::Conversation(id) => format!("conversation:{id}"),
            Scope::Session => "session".into(),
        }
    }
}

fn seen_key(scope: Scope, url: &str) -> String {
    format!("{}\n{url}", scope.key())
}

/// Why a call needs approval, sent with the approval request for the web app to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalReason {
    pub kind: &'static str,
    /// Untrusted content the conversation has read: `mail`, `web`, `shell`, `service`, `connector`.
    pub sources: Vec<&'static str>,
    /// Host the request goes to, where the tool has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    Allow,
    Ask(ApprovalReason),
    Refuse(&'static str),
}

impl Gate {
    /// For a tool the user always allows: untrusted content no longer stops it. Hidden
    /// characters still do; nobody can have meant to approve what they couldn't see.
    pub(crate) fn always_allowed(self) -> Gate {
        match self {
            Gate::Ask(reason) if reason.kind != "hidden_text" => Gate::Allow,
            Gate::Refuse(REFUSED_IN_RUN) => Gate::Allow,
            other => other,
        }
    }
}

#[derive(Default)]
pub(crate) struct Tracker {
    session: Exposure,
    runs: HashMap<String, Exposure>,
    run_order: VecDeque<String>,
    /// URLs from mail and web results, as `scope key\nurl`.
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    links_followed: HashMap<String, usize>,
}

impl Tracker {
    /// The exposure of a run or of the session; conversations are kept by the caller.
    pub(crate) fn exposure(&self, scope: Scope) -> Exposure {
        match scope {
            Scope::Run(run) => self.runs.get(run).copied().unwrap_or_default(),
            Scope::Session => self.session,
            Scope::Conversation(_) => Exposure::default(),
        }
    }

    /// `vetted` is [`Connector::vetted`](super::registry::Connector::vetted) for the tool's
    /// connector.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gate(
        &mut self,
        connector_id: &str,
        tool_name: &str,
        tool: Option<&ToolDefinition>,
        vetted: bool,
        input: &Value,
        scope: Scope,
        exposure: Exposure,
    ) -> Gate {
        let outbound = sends_data(connector_id, tool_name, tool, vetted);
        let hidden = outbound && hidden_text::contains_hidden(input);
        let writes = changes_files(connector_id, tool) && exposure.is_untrusted();
        let sends = outbound && exposure.is_risky();
        if !writes && !sends && !hidden {
            return Gate::Allow;
        }
        if matches!(scope, Scope::Run(_)) {
            return Gate::Refuse(if hidden {
                REFUSED_HIDDEN
            } else {
                REFUSED_IN_RUN
            });
        }
        if hidden {
            return Gate::Ask(ApprovalReason {
                kind: "hidden_text",
                sources: exposure.sources(),
                destination: destination(connector_id, input),
            });
        }
        if writes {
            return Gate::Ask(ApprovalReason {
                kind: "untrusted_write",
                sources: exposure.sources(),
                destination: None,
            });
        }
        if let Some(url) = plain_get_url(connector_id, tool_name, input) {
            let followed = self.links_followed.entry(scope.key()).or_default();
            if self.seen.contains(&seen_key(scope, &url)) && *followed < MAX_UNPROMPTED_LINKS {
                *followed += 1;
                return Gate::Allow;
            }
        }
        Gate::Ask(ApprovalReason {
            kind: "untrusted_content",
            sources: exposure.sources(),
            destination: destination(connector_id, input),
        })
    }

    /// Called after every call, failed ones included: an error can carry the remote's text.
    /// Returns what the call exposed, for the caller to add to a conversation.
    pub(crate) fn record(
        &mut self,
        connector_id: &str,
        tool_name: &str,
        scope: Scope,
        result: Option<&Value>,
    ) -> Exposure {
        let exposure = exposure_of(connector_id, tool_name);
        self.session.merge(exposure);
        if let Scope::Run(run) = scope {
            if !self.runs.contains_key(run) {
                if self.run_order.len() >= MAX_TRACKED_RUNS {
                    if let Some(oldest) = self.run_order.pop_front() {
                        self.runs.remove(&oldest);
                    }
                }
                self.run_order.push_back(run.to_string());
            }
            self.runs
                .entry(run.to_string())
                .or_default()
                .merge(exposure);
        }
        if let Some(result) = result.filter(|_| exposure.mail || exposure.web) {
            let mut urls = Vec::new();
            collect_urls(result, &mut urls);
            for url in urls {
                self.remember(seen_key(scope, &url));
            }
        }
        exposure
    }

    fn remember(&mut self, url: String) {
        if self.seen.contains(&url) {
            return;
        }
        if self.seen_order.len() >= MAX_SEEN_URLS {
            if let Some(oldest) = self.seen_order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        self.seen_order.push_back(url.clone());
        self.seen.insert(url);
    }
}

#[cfg(test)]
mod tests;
