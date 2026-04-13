/// Document store: maps doc_id → metadata so we can display results.
/// Keeps the original text around for snippet generation.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: u32,
    pub path: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentStore {
    docs: HashMap<u32, Document>,
    next_id: u32,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            next_id: 0,
        }
    }

    pub fn add(&mut self, path: String, title: String, body: String) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.docs.insert(
            id,
            Document {
                id,
                path,
                title,
                body,
            },
        );
        id
    }

    pub fn get(&self, id: u32) -> Option<&Document> {
        self.docs.get(&id)
    }

    pub fn count(&self) -> usize {
        self.docs.len()
    }

    /// Generate a context snippet around the first occurrence of any matched term.
    /// This is the "...three words before [MATCH] three words after..." preview.
    pub fn snippet(&self, doc_id: u32, matched_terms: &[String], context_words: usize) -> String {
        let doc = match self.get(doc_id) {
            Some(d) => d,
            None => return String::new(),
        };

        let lower = doc.body.to_lowercase();
        let words: Vec<&str> = doc.body.split_whitespace().collect();
        let lower_words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();

        // Find first matching word position
        let mut best_pos = None;
        for (i, lw) in lower_words.iter().enumerate() {
            let cleaned: String = lw.chars().filter(|c| c.is_alphanumeric()).collect();
            for term in matched_terms {
                if cleaned.contains(term.as_str()) || term.contains(cleaned.as_str()) {
                    best_pos = Some(i);
                    break;
                }
            }
            if best_pos.is_some() {
                break;
            }
        }

        let center = best_pos.unwrap_or(0);
        let start = center.saturating_sub(context_words);
        let end = (center + context_words + 1).min(words.len());

        let mut snippet = String::new();
        if start > 0 {
            snippet.push_str("...");
        }
        snippet.push_str(&words[start..end].join(" "));
        if end < words.len() {
            snippet.push_str("...");
        }
        snippet
    }
}
