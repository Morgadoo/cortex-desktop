//! Workflows: named, reusable sequences of steps, picked from the composer's + menu.
//!
//! Choosing one runs it on the next message: the model gets the steps in order, tracks them as
//! its plan, and works through them with whatever tools the conversation has. A workflow is a
//! recipe the user can read and edit, not code.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::chat::store::{chat_dir, valid_id, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    feature = "typescript",
    derive(ts_rs::TS),
    ts(rename = "WorkflowFields")
)]
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
            "builtin-image-to-video",
            "Image to video",
            "Design a still, then bring it to life as a short clip.",
            &[
                "Write a detailed image prompt from the user's idea (subject, style, light, framing) and make it with generate_image.",
                "Show the image and, if the user wants changes, edit it with generate_image using it as the reference.",
                "Animate the chosen image with generate_video, passing it as the reference and describing the motion and camera.",
                "Write a one-line caption and say where the image and the clip are saved.",
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
    let mut mine: Vec<Workflow> = crate::chat::at_rest::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for w in &mut mine {
        w.builtin = false;
    }
    let hidden: Vec<String> = mine.iter().map(|w| w.name.to_lowercase()).collect();
    mine.extend(
        builtins()
            .into_iter()
            .filter(|b| !hidden.contains(&b.name.to_lowercase())),
    );
    mine
}

fn save_mine(config_dir: &Path, workflows: &[Workflow]) -> Result<(), String> {
    let mine: Vec<&Workflow> = workflows.iter().filter(|w| !w.builtin).collect();
    write_json(&path(config_dir), &mine).map_err(String::from)
}

pub fn save(config_dir: &Path, mut workflow: Workflow) -> Result<Vec<Workflow>, String> {
    workflow.name = crate::support::text::clip(workflow.name.trim(), 80);
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
        Some(existing) => {
            crate::chat::versions::keep(
                config_dir,
                "workflows",
                &workflow.id,
                &*existing,
                &workflow,
            )?;
            *existing = workflow;
        }
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
        let brief = start
            .iter()
            .find(|w| w.name == "Research brief")
            .unwrap()
            .clone();
        let edited = Workflow {
            steps: vec!["Only one step".into()],
            ..brief
        };
        let all = save(dir.path(), edited).unwrap();
        assert!(!all[0].builtin);
        assert_eq!(all[0].steps, ["Only one step"]);
        assert_eq!(all.iter().filter(|w| w.name == "Research brief").count(), 1);
        let id = all[0].id.clone();
        let all = delete(dir.path(), &id).unwrap();
        assert!(all.iter().any(|w| w.name == "Research brief" && w.builtin));
        assert!(save(
            dir.path(),
            Workflow {
                id: String::new(),
                name: " ".into(),
                description: String::new(),
                steps: vec![],
                builtin: false
            }
        )
        .is_err());
    }
}

/// A workflow as the app shows it: with its production version, when one is marked.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    feature = "typescript",
    derive(ts_rs::TS),
    ts(export, rename = "Workflow")
)]
pub struct WorkflowView {
    #[serde(flatten)]
    pub workflow: Workflow,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub production: Option<Live>,
}

/// The production version's parts that run.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(rename = "LiveWorkflow"))]
pub struct Live {
    pub at: chrono::DateTime<chrono::Utc>,
    pub name: String,
    pub steps: Vec<String>,
}

pub fn views(config_dir: &Path, all: Vec<Workflow>) -> Vec<WorkflowView> {
    all.into_iter()
        .map(|workflow| {
            let production = if workflow.builtin {
                None
            } else {
                crate::chat::versions::production(config_dir, "workflows", &workflow.id)
            }
            .and_then(|p| {
                let w: Workflow = serde_json::from_value(p.value).ok()?;
                Some(Live {
                    at: p.at,
                    name: w.name,
                    steps: w.steps,
                })
            });
            WorkflowView {
                workflow,
                production,
            }
        })
        .collect()
}

/// The prompt asking a model to design a workflow, or to change `current` as `request` says.
pub fn draft_prompt(request: &str, current: Option<&Workflow>) -> String {
    let base = match current {
        Some(w) => format!(
            "Here is a workflow as JSON:\n{}\n\nRevise it as the user asks: {request}\nKeep what they did not ask to change.",
            serde_json::json!({ "name": w.name, "description": w.description, "steps": w.steps })
        ),
        None => format!("Design a workflow, a reusable recipe of steps an AI assistant follows in order, for this:\n\n{request}"),
    };
    format!(
        "{base}\n\nReply with only a JSON object, no prose and no code fence, with:\n\
         - name: 1 to 4 words\n\
         - description: one line saying what it is for\n\
         - steps: 3 to 8 imperative steps, one action each, naming the tools to use when it \
           matters (web_search, canvas, delegate, run_code, schedule_task…)\n\
         Where a step needs something from the user each time it runs, write an input as \
         {{{{name:type}}}} with type one of text, long, number, date, time, url, email, or a choice \
         as {{{{name|option a|option b}}}}. Use inputs only for what changes between runs."
    )
}

/// Reads a model's draft workflow.
pub fn parse_draft(text: &str) -> Result<Workflow, String> {
    let start = text
        .find('{')
        .ok_or("The model did not return a workflow.")?;
    let end = text
        .rfind('}')
        .ok_or("The model did not return a workflow.")?;
    let value: serde_json::Value = serde_json::from_str(&text[start..=end])
        .map_err(|_| "The model's draft was not valid JSON; try again.".to_string())?;
    let field = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let steps: Vec<String> = value
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if field("name").is_empty() || steps.is_empty() {
        return Err("The draft had no name or steps; try again.".into());
    }
    Ok(Workflow {
        id: String::new(),
        name: field("name"),
        description: field("description"),
        steps,
        builtin: false,
    })
}

#[cfg(test)]
mod draft_tests {
    use super::*;

    #[test]
    fn drafts_read_and_production_shows() {
        let w = parse_draft("Sure: {\"name\": \"Weekly report\", \"description\": \"Friday status\", \"steps\": [\"Gather {{week:number}} notes\", \" \", \"Write it\"]}").unwrap();
        assert_eq!(w.steps, ["Gather {{week:number}} notes", "Write it"]);
        assert!(parse_draft("{\"name\": \"x\", \"steps\": []}").is_err());
        let prompt = draft_prompt("make it shorter", Some(&w));
        assert!(
            prompt.contains("\"Weekly report\"")
                && prompt.contains("make it shorter")
                && prompt.contains("{{name:type}}"),
            "{prompt}"
        );

        let dir = tempfile::tempdir().unwrap();
        let all = save(dir.path(), w).unwrap();
        let mine = all.iter().find(|w| !w.builtin).unwrap().clone();
        assert!(views(dir.path(), all.clone())
            .iter()
            .all(|v| v.production.is_none()));
        crate::chat::versions::set_production(
            dir.path(),
            "workflows",
            &mine.id,
            Some(serde_json::to_value(&mine).unwrap()),
        )
        .unwrap();
        let mut edited = mine.clone();
        edited.steps = vec!["Draft step".into()];
        let all = save(dir.path(), edited).unwrap();
        let view = views(dir.path(), all)
            .into_iter()
            .find(|v| v.workflow.id == mine.id)
            .unwrap();
        assert_eq!(
            view.workflow.steps,
            ["Draft step"],
            "the editor sees the draft"
        );
        assert_eq!(
            view.production.unwrap().steps,
            mine.steps,
            "runs use production"
        );
    }
}
