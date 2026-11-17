//! Workflows: named, reusable sequences of steps, picked from the composer's + menu.
//!
//! Choosing one runs it on the next message: the model gets the steps in order, tracks them as
//! its plan, and works through them with whatever tools the conversation has. A workflow is a
//! recipe the user can read and edit, not code.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::store::{chat_dir, valid_id, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<String>,
    /// Ships with the app; editing one saves a copy of your own.
    #[serde(default)]
    pub builtin: bool,
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("workflows.json")
}

fn builtins() -> Vec<Workflow> {
    let make = |id: &str, name: &str, description: &str, steps: &[&str]| Workflow {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        steps: steps.iter().map(|s| s.to_string()).collect(),
        builtin: true,
    };
    vec![
        make(
            "builtin-research-brief",
            "Research brief",
            "Research a topic on the web and write a sourced brief in a canvas.",
            &[
                "Split the topic into 3–5 questions worth answering.",
                "Search the web for each question (in parallel with delegate when there are several).",
                "Cross-check the important facts in at least two sources.",
                "Write a brief in a canvas: summary, findings with citations, open questions.",
            ],
        ),
        make(
            "builtin-meeting-notes",
            "Meeting notes to actions",
            "Turn rough notes or a transcript into decisions, owners and next steps.",
            &[
                "Read the notes and list the decisions that were made.",
                "Extract every action item with its owner and due date (ask if an owner is unclear).",
                "Write a short summary email the user can send to attendees.",
            ],
        ),
        make(
            "builtin-code-review",
            "Review my changes",
            "Review the uncommitted changes in a coding session.",
            &[
                "Look at the uncommitted changes in the folder (git diff).",
                "Check each change for bugs, missed edge cases and unclear names.",
                "Run the project's tests or build if there are any.",
                "Report findings by severity, with file and line, and suggested fixes.",
            ],
        ),
        make(
            "builtin-data-analysis",
            "Analyze a dataset",
            "Load a CSV or spreadsheet, explore it and chart what matters.",
            &[
                "Load the data with run_code and describe its columns, size and gaps.",
                "Compute the summary statistics that answer the user's question.",
                "Make one or two clear charts and save them as images.",
                "Explain the findings in plain language, with the numbers that support them.",
            ],
        ),
    ]
}

/// The user's workflows, then the built-in ones they have not replaced.
pub fn load(config_dir: &Path) -> Vec<Workflow> {
    let mut mine: Vec<Workflow> = std::fs::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for w in &mut mine {
        w.builtin = false;
    }
    let hidden: Vec<String> = mine.iter().map(|w| w.name.to_lowercase()).collect();
    mine.extend(builtins().into_iter().filter(|b| !hidden.contains(&b.name.to_lowercase())));
    mine
}

fn save_mine(config_dir: &Path, workflows: &[Workflow]) -> Result<(), String> {
    let mine: Vec<&Workflow> = workflows.iter().filter(|w| !w.builtin).collect();
    write_json(&path(config_dir), &mine)
}

pub fn save(config_dir: &Path, mut workflow: Workflow) -> Result<Vec<Workflow>, String> {
    workflow.name = workflow.name.trim().chars().take(80).collect();
    workflow.steps.retain(|s| !s.trim().is_empty());
    if workflow.name.is_empty() || workflow.steps.is_empty() {
        return Err("A workflow needs a name and at least one step.".into());
    }
    // Editing a built-in one saves a copy of your own under a new id.
    if workflow.builtin || !valid_id(&workflow.id) {
        workflow.id = uuid::Uuid::new_v4().to_string();
    }
    workflow.builtin = false;
    let mut all = load(config_dir);
    match all.iter_mut().find(|w| w.id == workflow.id) {
        Some(existing) => *existing = workflow,
        None => all.insert(0, workflow),
    }
    save_mine(config_dir, &all)?;
    Ok(load(config_dir))
}

pub fn delete(config_dir: &Path, id: &str) -> Result<Vec<Workflow>, String> {
    let mut all = load(config_dir);
    all.retain(|w| w.builtin || w.id != id);
    save_mine(config_dir, &all)?;
    Ok(load(config_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_workflows_come_first_and_can_replace_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let start = load(dir.path());
        assert!(start.iter().all(|w| w.builtin));
        let brief = start.iter().find(|w| w.name == "Research brief").unwrap().clone();
        let edited = Workflow { steps: vec!["Only one step".into()], ..brief };
        let all = save(dir.path(), edited).unwrap();
        assert!(!all[0].builtin);
        assert_eq!(all[0].steps, ["Only one step"]);
        assert_eq!(all.iter().filter(|w| w.name == "Research brief").count(), 1);
        let id = all[0].id.clone();
        let all = delete(dir.path(), &id).unwrap();
        assert!(all.iter().any(|w| w.name == "Research brief" && w.builtin));
        assert!(save(dir.path(), Workflow { id: String::new(), name: " ".into(), description: String::new(), steps: vec![], builtin: false }).is_err());
    }
}
