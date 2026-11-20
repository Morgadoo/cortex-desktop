//! A model leaderboard from your own comparisons, as Open WebUI's arena keeps.
//!
//! Each time a comparison ends with "Keep this answer", the kept model wins against every other
//! model in it. Ratings are Elo (start 1000, K 32), replayed from the stored results in order, so
//! they can always be recomputed. Results live in `chat/arena.json`.

use crate::support::settings_file::SettingsFile;
use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const START: f64 = 1000.0;
const K: f64 = 32.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub winner: String,
    pub losers: Vec<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Standing {
    pub model: String,
    pub rating: i64,
    pub wins: u32,
    pub losses: u32,
}

const ARENA_FILE: SettingsFile<Vec<Match>> = SettingsFile::sealed("chat/arena.json");

pub fn load(config_dir: &Path) -> Vec<Match> {
    ARENA_FILE.load(config_dir)
}

/// Records a comparison's result.
pub fn record(config_dir: &Path, winner: &str, losers: &[String]) -> Result<(), String> {
    let losers: Vec<String> = losers
        .iter()
        .filter(|l| l.as_str() != winner)
        .cloned()
        .collect();
    if winner.trim().is_empty() || losers.is_empty() {
        return Err("A comparison needs a winner and at least one other model.".into());
    }
    let mut all = load(config_dir);
    all.push(Match {
        winner: winner.into(),
        losers,
        at: Utc::now(),
    });
    ARENA_FILE.save(config_dir, &all).map_err(String::from)
}

/// Ratings from the results, best first.
pub fn standings(matches: &[Match]) -> Vec<Standing> {
    let mut rating: HashMap<String, f64> = HashMap::new();
    let mut wins: HashMap<String, u32> = HashMap::new();
    let mut losses: HashMap<String, u32> = HashMap::new();
    for m in matches {
        for loser in &m.losers {
            let rw = *rating.get(&m.winner).unwrap_or(&START);
            let rl = *rating.get(loser).unwrap_or(&START);
            let expected = 1.0 / (1.0 + 10f64.powf((rl - rw) / 400.0));
            rating.insert(m.winner.clone(), rw + K * (1.0 - expected));
            rating.insert(loser.clone(), rl - K * (1.0 - expected));
            *wins.entry(m.winner.clone()).or_default() += 1;
            *losses.entry(loser.clone()).or_default() += 1;
        }
    }
    let mut out: Vec<Standing> = rating
        .into_iter()
        .map(|(model, r)| Standing {
            wins: *wins.get(&model).unwrap_or(&0),
            losses: *losses.get(&model).unwrap_or(&0),
            rating: r.round() as i64,
            model,
        })
        .collect();
    out.sort_by(|a, b| b.rating.cmp(&a.rating).then(a.model.cmp(&b.model)));
    out
}

pub fn reset(config_dir: &Path) -> Result<(), String> {
    ARENA_FILE
        .save(config_dir, &Vec::<Match>::new())
        .map_err(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wins_raise_ratings_in_order() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), "a/x", &["b/y".into(), "c/z".into()]).unwrap();
        record(dir.path(), "a/x", &["b/y".into()]).unwrap();
        record(dir.path(), "b/y", &["c/z".into()]).unwrap();
        assert!(
            record(dir.path(), "a/x", &["a/x".into()]).is_err(),
            "a model cannot beat itself"
        );
        let board = standings(&load(dir.path()));
        assert_eq!(
            board.iter().map(|s| s.model.as_str()).collect::<Vec<_>>(),
            ["a/x", "b/y", "c/z"]
        );
        assert_eq!((board[0].wins, board[0].losses), (3, 0));
        assert_eq!((board[1].wins, board[1].losses), (1, 2));
        assert!(board[0].rating > 1000 && board[2].rating < 1000);
        // Elo moves points around; the total stays the same.
        assert_eq!(board.iter().map(|s| s.rating).sum::<i64>(), 3000);
        reset(dir.path()).unwrap();
        assert!(standings(&load(dir.path())).is_empty());
    }
}
