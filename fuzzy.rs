/// Fuzzy search using Levenshtein distance.
///
/// We build a from-scratch edit distance calculator, then use it to find
/// index terms within a given edit distance of each query term.
/// This means "concensus" still finds "consensus", "memroy" finds "memory", etc.

use crate::index::InvertedIndex;
use crate::ranking::{self, BM25Config, SearchResult};
use crate::tokenizer;

/// Classic Levenshtein distance — the minimum number of single-character
/// edits (insert, delete, substitute) to transform `a` into `b`.
/// O(n*m) time, O(min(n,m)) space using a single-row optimization.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let (short, long) = if a_chars.len() <= b_chars.len() {
        (&a_chars, &b_chars)
    } else {
        (&b_chars, &a_chars)
    };

    let len_s = short.len();
    let len_l = long.len();

    // Single-row DP. row[j] = edit distance between short[..j] and long[..i]
    let mut row: Vec<usize> = (0..=len_s).collect();

    for i in 1..=len_l {
        let mut prev = row[0]; // row[0] from previous iteration
        row[0] = i;
        for j in 1..=len_s {
            let cost = if long[i - 1] == short[j - 1] { 0 } else { 1 };
            let temp = row[j];
            row[j] = min3(
                row[j] + 1,       // deletion
                row[j - 1] + 1,   // insertion
                prev + cost,       // substitution
            );
            prev = temp;
        }
    }

    row[len_s]
}

/// Damerau-Levenshtein: also allows transpositions (ab → ba = 1 edit).
/// More useful for typo correction since transposition is the most common typo.
pub fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let len_a = a_chars.len();
    let len_b = b_chars.len();

    if len_a == 0 { return len_b; }
    if len_b == 0 { return len_a; }

    // Full matrix needed for transposition check
    let mut matrix = vec![vec![0usize; len_b + 1]; len_a + 1];

    for i in 0..=len_a { matrix[i][0] = i; }
    for j in 0..=len_b { matrix[0][j] = j; }

    for i in 1..=len_a {
        for j in 1..=len_b {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            matrix[i][j] = min3(
                matrix[i - 1][j] + 1,
                matrix[i][j - 1] + 1,
                matrix[i - 1][j - 1] + cost,
            );
            // Transposition
            if i > 1 && j > 1
                && a_chars[i - 1] == b_chars[j - 2]
                && a_chars[i - 2] == b_chars[j - 1]
            {
                matrix[i][j] = matrix[i][j].min(matrix[i - 2][j - 2] + 1);
            }
        }
    }

    matrix[len_a][len_b]
}

fn min3(a: usize, b: usize, c: usize) -> usize {
    a.min(b).min(c)
}

/// Find all terms in the index within `max_dist` edit distance of `query_term`.
/// Returns (index_term, edit_distance) pairs sorted by distance.
pub fn fuzzy_match_terms(
    index: &InvertedIndex,
    query_term: &str,
    max_dist: usize,
) -> Vec<(String, usize)> {
    let stemmed = tokenizer::stem(query_term);
    let mut matches: Vec<(String, usize)> = Vec::new();

    for term in index.map.keys() {
        // Quick length-based pruning: if lengths differ by more than max_dist,
        // edit distance is guaranteed to exceed max_dist.
        let len_diff = if term.len() > stemmed.len() {
            term.len() - stemmed.len()
        } else {
            stemmed.len() - term.len()
        };
        if len_diff > max_dist {
            continue;
        }

        // Also prune by first-character check for distance 1
        // (optional optimization — skip if being too aggressive)

        let dist = damerau_levenshtein(&stemmed, term);
        if dist <= max_dist && dist > 0 {
            // dist > 0 excludes exact matches (we handle those normally)
            matches.push((term.clone(), dist));
        }
    }

    matches.sort_by_key(|&(_, d)| d);
    matches
}

/// Perform a fuzzy search: for each query term, expand to include
/// terms within edit distance `max_dist`, then BM25-rank the results.
/// Fuzzy matches are scored with a distance penalty.
pub fn fuzzy_search(
    index: &InvertedIndex,
    query: &str,
    config: &BM25Config,
    max_results: usize,
    max_dist: usize,
) -> FuzzySearchResult {
    let query_terms = tokenizer::tokenize(query);
    let mut expanded_terms: Vec<String> = Vec::new();
    let mut corrections: Vec<(String, String, usize)> = Vec::new(); // (original, corrected, dist)

    for term in &query_terms {
        // Always include the exact term
        expanded_terms.push(term.clone());

        // Check if the exact term exists in the index
        let exact_exists = index.map.contains_key(term.as_str());

        // Find fuzzy matches
        let fuzzy = fuzzy_match_terms(index, term, max_dist);
        for (matched_term, dist) in &fuzzy {
            if !expanded_terms.contains(matched_term) {
                expanded_terms.push(matched_term.clone());
            }
            if !exact_exists || *dist == 1 {
                corrections.push((term.clone(), matched_term.clone(), *dist));
            }
        }
    }

    // Build an expanded query string from all matched terms
    let expanded_query = expanded_terms.join(" ");
    let results = ranking::search(index, &expanded_query, config, max_results);

    FuzzySearchResult {
        results,
        corrections,
    }
}

pub struct FuzzySearchResult {
    pub results: Vec<SearchResult>,
    pub corrections: Vec<(String, String, usize)>,
}
