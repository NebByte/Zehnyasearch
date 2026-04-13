/// The inverted index: maps every stemmed term to the list of documents
/// that contain it, along with term frequency and positions.
/// Entirely hand-rolled — no external crates.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::tokenizer;

/// A single occurrence record: "term T appears in doc D at these positions"
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Posting {
    pub doc_id: u32,
    pub term_freq: u32,
    pub positions: Vec<u32>,
}

/// The inverted index itself.
#[derive(Debug, Serialize, Deserialize)]
pub struct InvertedIndex {
    /// term → sorted list of postings
    pub map: HashMap<String, Vec<Posting>>,
    /// doc_id → number of tokens in that document
    pub doc_lengths: HashMap<u32, u32>,
    /// total number of indexed documents
    pub doc_count: u32,
    /// sum of all document lengths (for avg calculation)
    pub total_length: u64,
}

impl InvertedIndex {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            doc_lengths: HashMap::new(),
            doc_count: 0,
            total_length: 0,
        }
    }

    /// Index a single document. This is the hot path — called once per doc.
    pub fn add_document(&mut self, doc_id: u32, text: &str) {
        let tokens = tokenizer::tokenize(text);
        let doc_len = tokens.len() as u32;

        self.doc_lengths.insert(doc_id, doc_len);
        self.doc_count += 1;
        self.total_length += doc_len as u64;

        // Build per-term positions map for this document
        let mut term_positions: HashMap<&str, Vec<u32>> = HashMap::new();
        for (pos, token) in tokens.iter().enumerate() {
            term_positions
                .entry(token.as_str())
                .or_default()
                .push(pos as u32);
        }

        // Merge into the global inverted index
        for (term, positions) in term_positions {
            let posting = Posting {
                doc_id,
                term_freq: positions.len() as u32,
                positions,
            };
            self.map
                .entry(term.to_string())
                .or_default()
                .push(posting);
        }
    }

    /// Get the postings list for a term (or empty slice).
    pub fn get_postings(&self, term: &str) -> &[Posting] {
        self.map.get(term).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Average document length across the entire corpus.
    pub fn avg_doc_length(&self) -> f64 {
        if self.doc_count == 0 {
            return 0.0;
        }
        self.total_length as f64 / self.doc_count as f64
    }

    /// How many documents contain this term.
    pub fn doc_frequency(&self, term: &str) -> u32 {
        self.map.get(term).map(|v| v.len() as u32).unwrap_or(0)
    }

    /// Memory stats for bragging rights
    pub fn stats(&self) -> IndexStats {
        let total_postings: usize = self.map.values().map(|v| v.len()).sum();
        IndexStats {
            unique_terms: self.map.len(),
            total_postings,
            documents: self.doc_count as usize,
            avg_doc_length: self.avg_doc_length(),
        }
    }
}

#[derive(Debug)]
pub struct IndexStats {
    pub unique_terms: usize,
    pub total_postings: usize,
    pub documents: usize,
    pub avg_doc_length: f64,
}

impl std::fmt::Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Documents: {}\nUnique terms: {}\nTotal postings: {}\nAvg doc length: {:.1} tokens",
            self.documents, self.unique_terms, self.total_postings, self.avg_doc_length
        )
    }
}
