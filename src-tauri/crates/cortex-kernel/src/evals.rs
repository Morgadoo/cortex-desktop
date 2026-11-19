//! Evaluation, the pure part: what a case expects, how an answer is graded, and how two runs of
//! a suite compare. Running cases is the app's (`chat/evals.rs`), through its agent loop.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Case {
    pub id: String,
    pub prompt: String,
    /// Tools a good answer calls.
    pub tools: Vec<String>,
    /// Tools a good answer does not call.
    pub not_tools: Vec<String>,
    /// Words or phrases a good answer contains.
    pub facts: Vec<String>,
    /// Saved from a rating, with nothing yet to check: skipped until the user adds expectations.
    pub needs_expectation: bool,
    pub note: String,
}

impl Case {
    pub fn checks(&self) -> usize {
        self.tools.len() + self.not_tools.len() + self.facts.len()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseResult {
    pub id: String,
    pub passed: bool,
    /// Checks met, out of 1.
    pub score: f64,
    /// The checks it failed, in words.
    pub missing: Vec<String>,
    pub tools_called: Vec<String>,
    pub skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteRun {
    pub suite: String,
    pub at: DateTime<Utc>,
    pub model: String,
    /// What was left out (`tool:web_search`, `skill:summarize`), for an ablation.
    pub without: Vec<String>,
    /// The mean case score, over the cases that ran.
    pub score: f64,
    pub cases: Vec<CaseResult>,
}

/// Grades one answer against its case.
pub fn grade(case: &Case, called: &[String], answer: &str) -> CaseResult {
    if case.needs_expectation || case.checks() == 0 {
        return CaseResult {
            id: case.id.clone(),
            passed: false,
            score: 0.0,
            missing: vec!["no expectations yet".into()],
            tools_called: called.to_vec(),
            skipped: true,
        };
    }
    let mut missing = Vec::new();
    for tool in &case.tools {
        if !called.contains(tool) {
            missing.push(format!("did not call {tool}"));
        }
    }
    for tool in &case.not_tools {
        if called.contains(tool) {
            missing.push(format!("called {tool}"));
        }
    }
    let lower = answer.to_lowercase();
    for fact in &case.facts {
        if !lower.contains(&fact.to_lowercase()) {
            missing.push(format!("does not say \"{fact}\""));
        }
    }
    let score = 1.0 - missing.len() as f64 / case.checks() as f64;
    CaseResult {
        id: case.id.clone(),
        passed: missing.is_empty(),
        score,
        missing,
        tools_called: called.to_vec(),
        skipped: false,
    }
}

/// What an ablation leaves out.
#[derive(Debug, Clone, Default)]
pub struct Without {
    pub tools: Vec<String>,
    pub skills: Vec<String>,
}

impl Without {
    /// From `tool:name` and `skill:name` items.
    pub fn parse(items: &[String]) -> Result<Self, String> {
        let mut out = Self::default();
        for item in items {
            match item.split_once(':') {
                Some(("tool", name)) => out.tools.push(name.to_string()),
                Some(("skill", name)) => out.skills.push(name.to_string()),
                _ => return Err(format!("`{item}`: write tool:<name> or skill:<name>")),
            }
        }
        Ok(out)
    }

    pub fn labels(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|t| format!("tool:{t}"))
            .chain(self.skills.iter().map(|s| format!("skill:{s}")))
            .collect()
    }
}

/// How a second run differs from a first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub before: f64,
    pub after: f64,
    pub better: Vec<String>,
    pub worse: Vec<String>,
}

impl Comparison {
    /// The second run is at least as good, case by case and overall.
    pub fn holds(&self) -> bool {
        self.worse.is_empty() && self.after + 1e-9 >= self.before
    }
}

pub fn compare(before: &SuiteRun, after: &SuiteRun) -> Comparison {
    let mut better = Vec::new();
    let mut worse = Vec::new();
    for b in &after.cases {
        let Some(a) = before.cases.iter().find(|a| a.id == b.id) else {
            continue;
        };
        if a.skipped || b.skipped {
            continue;
        }
        if b.score > a.score + 1e-9 {
            better.push(b.id.clone());
        } else if b.score + 1e-9 < a.score {
            worse.push(b.id.clone());
        }
    }
    Comparison {
        before: before.score,
        after: after.score,
        better,
        worse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(tools: &[&str], facts: &[&str]) -> Case {
        Case {
            id: "c".into(),
            prompt: "p".into(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            facts: facts.iter().map(|t| t.to_string()).collect(),
            ..Case::default()
        }
    }

    #[test]
    fn answers_are_graded_on_tools_and_facts() {
        let c = case(&["web_search"], &["Rust 1.99"]);
        let good = grade(
            &c,
            &["web_search".into()],
            "It is rust 1.99, released in September.",
        );
        assert!(good.passed && good.score == 1.0);
        let half = grade(&c, &[], "It is Rust 1.99.");
        assert!(!half.passed);
        assert_eq!(half.score, 0.5);
        assert_eq!(half.missing, vec!["did not call web_search".to_string()]);
        let mut not = case(&[], &[]);
        not.not_tools = vec!["delegate".into()];
        assert!(!grade(&not, &["delegate".into()], "").passed);
        let mut waiting = case(&[], &[]);
        waiting.needs_expectation = true;
        assert!(grade(&waiting, &[], "").skipped);
    }
    #[test]
    fn comparisons_name_the_cases_that_moved() {
        let result = |id: &str, score: f64| CaseResult {
            id: id.into(),
            passed: score == 1.0,
            score,
            missing: Vec::new(),
            tools_called: Vec::new(),
            skipped: false,
        };
        let run = |scores: &[(&str, f64)]| SuiteRun {
            suite: "s".into(),
            at: Utc::now(),
            model: "m".into(),
            without: Vec::new(),
            score: scores.iter().map(|s| s.1).sum::<f64>() / scores.len() as f64,
            cases: scores.iter().map(|(id, s)| result(id, *s)).collect(),
        };
        let c = compare(
            &run(&[("a", 1.0), ("b", 0.5)]),
            &run(&[("a", 0.5), ("b", 1.0)]),
        );
        assert_eq!(
            (c.better.clone(), c.worse.clone()),
            (vec!["b".to_string()], vec!["a".to_string()])
        );
        assert!(!c.holds());
        assert!(compare(&run(&[("a", 1.0)]), &run(&[("a", 1.0)])).holds());
    }
}
