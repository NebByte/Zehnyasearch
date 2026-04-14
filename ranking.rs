/// BM25 ranking — the algorithm that made Google possible.
/// We implement the Okapi BM25 formula from scratch.
///
/// Score(D, Q) = Σ IDF(qi) × [ f(qi,D) × (k1 + 1) ]
///                              ─────────────────────────
///                              f(qi,D) + k1 × (1 - b + b × |D|/avgdl)
///
/// Where:
///   f(qi,D)  = term frequency of qi in document D
///   |D|      = length of document D (in tokens)
///   avgdl    = average document length across the corpus
///   k1, b    = tuning parameters (1.2 and 0.75 are classics)

use crate::index::InvertedIndex;
use crate::store::DocumentStore;
use crate::tokenizer;

/// Tunable BM25 parameters
pub struct BM25Config {
    /// Controls term frequency saturation. Higher = slower saturation.
    pub k1: f64,
    /// Controls document length normalization. 0 = no normalization, 1 = full.
    pub b: f64,
}

impl Default for BM25Config {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// A single search result with its score and doc_id.
#[derive(Debug)]
pub struct SearchResult {
    pub doc_id: u32,
    pub score: f64,
    /// Which query terms actually matched (for highlighting)
    pub matched_terms: Vec<String>,
}

/// Compute BM25 IDF for a term.
/// Uses the smoothed variant: ln((N - n + 0.5) / (n + 0.5) + 1)
/// This avoids negative IDF for very common terms.
fn idf(doc_count: u32, doc_freq: u32) -> f64 {
    let n = doc_count as f64;
    let df = doc_freq as f64;
    ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
}

/// Run a BM25-ranked search. Returns results sorted by relevance (best first).
pub fn search(
    index: &InvertedIndex,
    query: &str,
    config: &BM25Config,
    max_results: usize,
) -> Vec<SearchResult> {
    let query_terms = tokenizer::tokenize(query);

    if query_terms.is_empty() {
        return vec![];
    }

    let avgdl = index.avg_doc_length();

    // Accumulate scores per document
    let mut scores: std::collections::HashMap<u32, (f64, Vec<String>)> =
        std::collections::HashMap::new();

    for term in &query_terms {
        let df = index.doc_frequency(term);
        if df == 0 {
            continue;
        }

        let term_idf = idf(index.doc_count, df);
        let postings = index.get_postings(term);

        for posting in postings {
            let tf = posting.term_freq as f64;
            let dl = *index.doc_lengths.get(&posting.doc_id).unwrap_or(&0) as f64;

            // The BM25 formula
            let numerator = tf * (config.k1 + 1.0);
            let denominator = tf + config.k1 * (1.0 - config.b + config.b * dl / avgdl);
            let term_score = term_idf * numerator / denominator;

            let entry = scores
                .entry(posting.doc_id)
                .or_insert((0.0, Vec::new()));
            entry.0 += term_score;
            if !entry.1.contains(term) {
                entry.1.push(term.clone());
            }
        }
    }

    // Sort by score descending
    let mut results: Vec<SearchResult> = scores
        .into_iter()
        .map(|(doc_id, (score, matched_terms))| SearchResult {
            doc_id,
            score,
            matched_terms,
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(max_results);
    results
}

/// Phrase search: only return docs where the query terms appear consecutively.
pub fn phrase_search(
    index: &InvertedIndex,
    query: &str,
    config: &BM25Config,
    max_results: usize,
) -> Vec<SearchResult> {
    let query_terms = tokenizer::tokenize(query);
    if query_terms.len() < 2 {
        return search(index, query, config, max_results);
    }

    // Find documents that contain ALL query terms
    let mut candidate_docs: Option<Vec<u32>> = None;
    for term in &query_terms {
        let postings = index.get_postings(term);
        let doc_ids: Vec<u32> = postings.iter().map(|p| p.doc_id).collect();
        candidate_docs = Some(match candidate_docs {
            None => doc_ids,
            Some(prev) => prev.into_iter().filter(|id| doc_ids.contains(id)).collect(),
        });
    }

    let candidates = candidate_docs.unwrap_or_default();

    // For each candidate, check if terms appear as a phrase
    let mut phrase_docs: Vec<u32> = Vec::new();

    for doc_id in candidates {
        // Get positions for each query term in this doc
        let positions: Vec<Vec<u32>> = query_terms
            .iter()
            .map(|term| {
                index
                    .get_postings(term)
                    .iter()
                    .find(|p| p.doc_id == doc_id)
                    .map(|p| p.positions.clone())
                    .unwrap_or_default()
            })
            .collect();

        // Check if there's a sequence where each term appears at consecutive positions
        if has_consecutive_positions(&positions) {
            phrase_docs.push(doc_id);
        }
    }

    // Now BM25-score just the phrase-matching docs (boost them)
    let all_results = search(index, query, config, max_results * 5);
    let mut filtered: Vec<SearchResult> = all_results
        .into_iter()
        .filter(|r| phrase_docs.contains(&r.doc_id))
        .collect();
    filtered.truncate(max_results);
    filtered
}

/// Rerank BM25 results with:
///   * title match boost (×1.6 if any query term is in the doc title)
///   * URL/path match boost (×1.2 if any query term is in the URL)
///   * PageRank blend — multiplies by `1 + alpha * ln(1 + pr * N)` so
///     the boost stays bounded even when pagerank spikes.
///
/// Call after any BM25-based search. Pass `pageranks = &[]` to skip the
/// PR component (e.g. when the corpus was indexed from local files).
pub fn rerank_results(
    results: &mut Vec<SearchResult>,
    store: &DocumentStore,
    pageranks: &[f32],
    query: &str,
    alpha: f64,
) {
    let q_tokens: std::collections::HashSet<String> =
        tokenizer::tokenize(query).into_iter().collect();
    if q_tokens.is_empty() {
        return;
    }

    let n = pageranks.len() as f64;

    for r in results.iter_mut() {
        let mut mult = 1.0;
        if let Some(doc) = store.get(r.doc_id) {
            let title_tokens: std::collections::HashSet<String> =
                tokenizer::tokenize(&doc.title).into_iter().collect();
            if q_tokens.iter().any(|t| title_tokens.contains(t)) {
                mult *= 1.6;
            }
            let path_tokens: std::collections::HashSet<String> =
                tokenizer::tokenize(&doc.path).into_iter().collect();
            if q_tokens.iter().any(|t| path_tokens.contains(t)) {
                mult *= 1.2;
            }
        }
        if n > 0.0 {
            let pr = pageranks.get(r.doc_id as usize).copied().unwrap_or(0.0) as f64;
            mult *= 1.0 + alpha * (1.0 + pr * n).ln();
        }
        r.score *= mult;
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
}

fn has_consecutive_positions(positions: &[Vec<u32>]) -> bool {
    if positions.is_empty() || positions[0].is_empty() {
        return false;
    }
    // For each starting position of the first term
    for &start in &positions[0] {
        let mut found = true;
        for (i, pos_list) in positions.iter().enumerate().skip(1) {
            if !pos_list.contains(&(start + i as u32)) {
                found = false;
                break;
            }
        }
        if found {
            return true;
        }
    }
    false
}
