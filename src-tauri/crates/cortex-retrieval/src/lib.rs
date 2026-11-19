//! One retriever for what the assistant looks things up in: library passages, knowledge pages,
//! tools to load, skills to offer. One tokenizer and one BM25 ranking, so a fix or a better
//! scorer reaches all of them at once.
//!
//! Semantic search with embeddings, which needs an embedding model, stays with the libraries
//! (`embeddings.rs`), where it is fused with this ranking by rank.

use std::collections::HashMap;

/// Lowercase words without accents, of two characters or more.
pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(|w| {
            w.to_lowercase()
                .chars()
                .map(|c| match c {
                    'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
                    'é' | 'è' | 'ê' | 'ë' => 'e',
                    'í' | 'ì' | 'î' | 'ï' => 'i',
                    'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
                    'ú' | 'ù' | 'û' | 'ü' => 'u',
                    'ç' => 'c',
                    'ñ' => 'n',
                    other => other,
                })
                .collect()
        })
        .collect()
}

/// Documents ready to rank, each given as its words. Repeat a field's words to weigh it more
/// (a title twice, a tool's name three times).
pub struct Corpus {
    terms: Vec<HashMap<String, f64>>,
    lens: Vec<f64>,
    avg: f64,
    /// A query word also matches the longer words it starts (`email` finds `emails`).
    prefixes: bool,
}

impl Corpus {
    pub fn new(docs: impl IntoIterator<Item = Vec<String>>) -> Self {
        let mut terms = Vec::new();
        let mut lens = Vec::new();
        for words in docs {
            let mut counts: HashMap<String, f64> = HashMap::new();
            for w in &words {
                *counts.entry(w.clone()).or_insert(0.0) += 1.0;
            }
            lens.push(words.len() as f64);
            terms.push(counts);
        }
        let avg = if lens.is_empty() {
            0.0
        } else {
            lens.iter().sum::<f64>() / lens.len() as f64
        };
        Self {
            terms,
            lens,
            avg,
            prefixes: false,
        }
    }

    /// Lets query words of three letters or more match as prefixes.
    pub fn with_prefixes(mut self) -> Self {
        self.prefixes = true;
        self
    }

    /// How often `term` occurs in document `i`.
    fn tf(&self, i: usize, term: &str) -> f64 {
        let exact = self.terms[i].get(term).copied().unwrap_or(0.0);
        if !self.prefixes || term.chars().count() < 3 {
            return exact;
        }
        self.terms[i]
            .iter()
            .filter(|(w, _)| w.len() > term.len() && w.starts_with(term))
            .map(|(_, n)| n)
            .sum::<f64>()
            + exact
    }

    /// BM25 scores for `query_terms`, best first, without the documents that match nothing.
    pub fn rank(&self, query_terms: &[String]) -> Vec<(usize, f64)> {
        if self.terms.is_empty() || query_terms.is_empty() {
            return Vec::new();
        }
        let n = self.terms.len() as f64;
        let (k1, b) = (1.5, 0.75);
        let idf: Vec<f64> = query_terms
            .iter()
            .map(|q| {
                let df = (0..self.terms.len())
                    .filter(|i| self.tf(*i, q) > 0.0)
                    .count() as f64;
                ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
            })
            .collect();
        let mut scored: Vec<(usize, f64)> = (0..self.terms.len())
            .map(|i| {
                let score = query_terms
                    .iter()
                    .zip(&idf)
                    .map(|(q, idf)| {
                        let tf = self.tf(i, q);
                        idf * tf * (k1 + 1.0)
                            / (tf + k1 * (1.0 - b + b * self.lens[i] / self.avg.max(1.0)))
                    })
                    .sum::<f64>();
                (i, score)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored
    }

    /// [`rank`](Self::rank) for a query written as text.
    pub fn search(&self, query: &str) -> Vec<(usize, f64)> {
        self.rank(&tokens(query))
    }
}

/// Reciprocal rank fusion: several rankings of the same items (by index), merged into one. An
/// item high in more than one list rises; ties keep the first list's order.
pub fn fuse_ranks(lists: &[Vec<usize>]) -> Vec<usize> {
    const K: f64 = 60.0;
    let mut scores: Vec<(usize, f64, (usize, usize))> = Vec::new();
    for (which, list) in lists.iter().enumerate() {
        for (rank, item) in list.iter().enumerate() {
            let add = 1.0 / (K + rank as f64 + 1.0);
            match scores.iter_mut().find(|(i, _, _)| i == item) {
                Some(entry) => entry.1 += add,
                None => scores.push((*item, add, (which, rank))),
            }
        }
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)));
    scores.into_iter().map(|(i, _, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rare_words_weigh_more_and_weighted_fields_count() {
        let corpus = Corpus::new([
            tokens("the report on the energy market"),
            tokens("the the the market"),
            tokens("energy energy prices"),
        ]);
        let ranked = corpus.search("energy report");
        assert_eq!(ranked[0].0, 0, "{ranked:?}");
        assert!(ranked.iter().all(|(i, _)| *i != 1));
        assert!(corpus.search("nothing here").is_empty());
    }

    #[test]
    fn prefixes_match_only_when_asked() {
        let docs = || [tokens("send emails to a contact"), tokens("read files")];
        assert!(Corpus::new(docs()).search("email").is_empty());
        let ranked = Corpus::new(docs()).with_prefixes().search("email");
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, 0);
        // Two letters are too few to match as a prefix.
        assert!(Corpus::new(docs()).with_prefixes().search("re").is_empty());
    }

    #[test]
    fn rankings_fuse_by_agreement() {
        // 2 is second in both lists, so it beats items first in only one.
        assert_eq!(fuse_ranks(&[vec![0, 2, 1], vec![3, 2, 4]])[0], 2);
        assert_eq!(fuse_ranks(&[vec![5, 6]]), vec![5, 6]);
        assert!(fuse_ranks(&[]).is_empty());
    }

    #[test]
    fn tokens_ignore_case_and_accents() {
        assert_eq!(
            tokens("Regulação da ERSE, 2026!"),
            vec!["regulacao", "da", "erse", "2026"]
        );
    }
}
