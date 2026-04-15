/// BK-Tree: a metric tree for fast approximate string matching.
///
/// The key insight: edit distance obeys the triangle inequality.
/// If we know distance(query, nodeA) = d, then any word within distance k of
/// the query MUST have distance in [d-k, d+k] from nodeA. This lets us
/// prune huge branches of the tree.
///
/// Built from scratch. This is the same structure used by production spell
/// checkers — most people have never heard of it.

use std::collections::HashMap;
use crate::fuzzy;

/// A node in the BK-Tree.
struct BKNode {
    word: String,
    /// children indexed by their edit distance from this node's word
    children: HashMap<usize, BKNode>,
}

/// The BK-Tree itself.
pub struct BKTree {
    root: Option<BKNode>,
    size: usize,
}

impl BKTree {
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
        }
    }

    /// Build a BK-Tree from a list of words (e.g., all terms in the index).
    pub fn from_words(words: &[String]) -> Self {
        let mut tree = Self::new();
        for word in words {
            tree.insert(word.clone());
        }
        tree
    }

    /// Insert a word into the tree.
    pub fn insert(&mut self, word: String) {
        self.size += 1;

        if self.root.is_none() {
            self.root = Some(BKNode {
                word,
                children: HashMap::new(),
            });
            return;
        }

        // Walk down the tree to find the right spot
        let mut current = self.root.as_mut().unwrap();
        loop {
            let dist = fuzzy::damerau_levenshtein(&current.word, &word);

            if dist == 0 {
                return; // duplicate word
            }

            if current.children.contains_key(&dist) {
                current = current.children.get_mut(&dist).unwrap();
            } else {
                current.children.insert(
                    dist,
                    BKNode {
                        word,
                        children: HashMap::new(),
                    },
                );
                return;
            }
        }
    }

    /// Find all words within `max_dist` edits of `query`.
    /// Returns (word, distance) pairs sorted by distance.
    ///
    /// This is the magic: the triangle inequality prunes most of the tree.
    /// Average case visits O(log n) nodes instead of O(n).
    pub fn find(&self, query: &str, max_dist: usize) -> Vec<(String, usize)> {
        let mut results = Vec::new();

        if let Some(root) = &self.root {
            let mut stack = vec![root];

            while let Some(node) = stack.pop() {
                let dist = fuzzy::damerau_levenshtein(query, &node.word);

                if dist <= max_dist {
                    results.push((node.word.clone(), dist));
                }

                // Triangle inequality pruning:
                // Only visit children whose distance key is in [dist-max_dist, dist+max_dist]
                let low = dist.saturating_sub(max_dist);
                let high = dist + max_dist;

                for (&child_dist, child_node) in &node.children {
                    if child_dist >= low && child_dist <= high {
                        stack.push(child_node);
                    }
                }
            }
        }

        results.sort_by_key(|&(_, d)| d);
        results
    }

    /// Suggest corrections: returns the best "did you mean?" candidates.
    /// Filters out exact matches and limits results.
    pub fn suggest(&self, query: &str, max_dist: usize, max_suggestions: usize) -> Vec<Suggestion> {
        let matches = self.find(query, max_dist);

        matches
            .into_iter()
            .filter(|(word, dist)| *dist > 0 && word != query) // skip exact match
            .take(max_suggestions)
            .map(|(word, distance)| Suggestion { word, distance })
            .collect()
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub word: String,
    pub distance: usize,
}

impl std::fmt::Display for Suggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (edit distance: {})", self.word, self.distance)
    }
}

/// Spelling corrector: wraps the BK-Tree and adds query-level correction.
/// Given a multi-word query, corrects each word independently and builds
/// a "did you mean: ...?" suggestion.
pub struct SpellCorrector {
    tree: BKTree,
}

impl SpellCorrector {
    pub fn new(vocabulary: &[String]) -> Self {
        Self {
            tree: BKTree::from_words(vocabulary),
        }
    }

    /// Correct a full query string. Returns None if no corrections needed.
    pub fn correct_query(&self, query: &str) -> Option<CorrectedQuery> {
        let words: Vec<&str> = query
            .split_whitespace()
            .filter(|w| w.len() > 1)
            .collect();

        let mut corrected_words = Vec::new();
        let mut any_corrections = false;

        for word in &words {
            let lower = word.to_lowercase();
            let stemmed = crate::tokenizer::stem(&lower);

            // Check if the stemmed term exists in the vocabulary
            let suggestions = self.tree.find(&stemmed, 0);
            if !suggestions.is_empty() {
                // Exact match exists — keep original
                corrected_words.push(word.to_string());
                continue;
            }

            // No exact match — find closest word
            let nearby = self.tree.suggest(&stemmed, 2, 1);
            if let Some(best) = nearby.first() {
                corrected_words.push(best.word.clone());
                any_corrections = true;
            } else {
                corrected_words.push(word.to_string());
            }
        }

        if any_corrections {
            Some(CorrectedQuery {
                original: query.to_string(),
                corrected: corrected_words.join(" "),
            })
        } else {
            None
        }
    }

    /// Get raw suggestions for a single term.
    pub fn suggest_term(&self, term: &str, max_dist: usize) -> Vec<Suggestion> {
        let stemmed = crate::tokenizer::stem(&term.to_lowercase());
        self.tree.suggest(&stemmed, max_dist, 5)
    }
}

#[derive(Debug)]
pub struct CorrectedQuery {
    pub original: String,
    pub corrected: String,
}

impl std::fmt::Display for CorrectedQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Did you mean: \"{}\"?", self.corrected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bk_tree_exact() {
        let words: Vec<String> = vec![
            "hello", "help", "hell", "world", "word",
        ].into_iter().map(String::from).collect();

        let tree = BKTree::from_words(&words);
        let results = tree.find("hello", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "hello");
    }

    #[test]
    fn test_bk_tree_fuzzy() {
        let words: Vec<String> = vec![
            "hello", "help", "hell", "world", "word",
        ].into_iter().map(String::from).collect();

        let tree = BKTree::from_words(&words);
        let results = tree.find("helo", 1);
        assert!(results.iter().any(|(w, _)| w == "hello"));
    }

    #[test]
    fn test_bk_tree_suggest() {
        let words: Vec<String> = vec![
            "consensus", "concurrent", "connection", "constant",
        ].into_iter().map(String::from).collect();

        let tree = BKTree::from_words(&words);
        let suggestions = tree.suggest("concensus", 2, 3);
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].word, "consensus");
    }
}
