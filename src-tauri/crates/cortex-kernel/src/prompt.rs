//! The system prompt, assembled from named sections instead of appended strings.
//!
//! Each section has a priority. The prompt keeps to a budget (a share of the model's window, so
//! the conversation and the tools keep room): when it is over, the least important sections drop
//! first, largest first. What was kept and how big each part is goes with the conversation, so
//! the context view shows the same parts the model received.

use serde::{Deserialize, Serialize};

/// What the turn cannot work without: who the assistant is, the user's instructions, the mode.
pub const ESSENTIAL: u8 = 0;
/// What helps with this message: memories, skills, the project, tools' guidance.
pub const USEFUL: u8 = 1;
/// What may help: knowledge pages, the team, canvases, the project's other chats.
pub const OPTIONAL: u8 = 2;

/// The share of the model's window the system prompt may take.
const BUDGET_SHARE: f64 = 0.25;
/// The least it may take, however small the window.
const MIN_BUDGET: u64 = 4_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub name: &'static str,
    pub priority: u8,
    pub text: String,
}

/// A section's size as the context view shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub name: String,
    pub tokens: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prompt {
    sections: Vec<Section>,
}

fn tokens(text: &str) -> u64 {
    text.len() as u64 / 4
}

impl Prompt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a section; an empty one is left out. A second section of the same name joins the first.
    pub fn push(&mut self, name: &'static str, priority: u8, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        match self.sections.iter_mut().find(|s| s.name == name) {
            Some(section) => section.text.push_str(&text),
            None => self.sections.push(Section {
                name,
                priority,
                text,
            }),
        }
    }

    /// Replaces everything with one section (a worker's own brief).
    pub fn replace_all(&mut self, name: &'static str, text: String) {
        self.sections = vec![Section {
            name,
            priority: ESSENTIAL,
            text,
        }];
    }

    /// The whole prompt, every section in order.
    pub fn render(&self) -> String {
        self.sections.iter().map(|s| s.text.as_str()).collect()
    }

    /// The prompt within the budget for a model with `window` tokens: optional sections drop
    /// first, then useful ones, the largest first; essential ones always stay. Returns the text,
    /// the parts kept, and the names of the parts dropped.
    pub fn fit(&self, window: u64) -> (String, Vec<Part>, Vec<&'static str>) {
        let budget = ((window as f64 * BUDGET_SHARE) as u64).max(MIN_BUDGET);
        let mut kept: Vec<&Section> = self.sections.iter().collect();
        let mut dropped = Vec::new();
        while kept.iter().map(|s| tokens(&s.text)).sum::<u64>() > budget {
            let Some((i, _)) = kept
                .iter()
                .enumerate()
                .filter(|(_, s)| s.priority > ESSENTIAL)
                .max_by_key(|(_, s)| (s.priority, tokens(&s.text)))
            else {
                break;
            };
            dropped.push(kept.remove(i).name);
        }
        let parts = kept
            .iter()
            .map(|s| Part {
                name: s.name.to_string(),
                tokens: tokens(&s.text),
            })
            .collect();
        (
            kept.iter().map(|s| s.text.as_str()).collect(),
            parts,
            dropped,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_render_in_order_and_join_by_name() {
        let mut p = Prompt::new();
        p.push("identity", ESSENTIAL, "You are Cortex.");
        p.push("skills", USEFUL, " Skills: a.");
        p.push("empty", OPTIONAL, "   ");
        p.push("skills", USEFUL, " And b.");
        assert_eq!(p.render(), "You are Cortex. Skills: a. And b.");
        assert_eq!(p.sections.len(), 2);
    }

    #[test]
    fn over_budget_the_least_important_and_largest_go_first() {
        let mut p = Prompt::new();
        p.push("identity", ESSENTIAL, "x".repeat(8_000));
        p.push("memories", USEFUL, "m".repeat(4_000));
        p.push("team", OPTIONAL, "t".repeat(2_000));
        p.push("knowledge", OPTIONAL, "k".repeat(6_000));
        // A 16k window gives the 4,000-token floor: identity (2,000) and memories (1,000) fit.
        let (text, parts, dropped) = p.fit(16_000);
        assert_eq!(dropped, vec!["knowledge"]);
        assert_eq!(parts.len(), 3);
        assert_eq!(text.len(), 14_000);
        // A big window keeps everything.
        let (_, _, dropped) = p.fit(1_000_000);
        assert!(dropped.is_empty());
        // Essential sections stay even when they alone are over the budget.
        let mut big = Prompt::new();
        big.push("identity", ESSENTIAL, "x".repeat(20_000));
        big.push("team", OPTIONAL, "t".repeat(400));
        let (_, parts, dropped) = big.fit(1);
        assert_eq!(dropped, vec!["team"]);
        assert_eq!(parts.len(), 1);
    }
}
