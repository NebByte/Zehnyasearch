/// Hand-rolled tokenizer: lowercase, split, stop-word removal, and a
/// simplified Porter stemmer. No crates — every rule is ours.

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for",
    "of", "with", "by", "from", "is", "it", "as", "was", "are", "were",
    "been", "be", "have", "has", "had", "do", "does", "did", "will",
    "would", "could", "should", "may", "might", "shall", "can", "this",
    "that", "these", "those", "i", "you", "he", "she", "we", "they",
    "me", "him", "her", "us", "them", "my", "your", "his", "its", "our",
    "their", "what", "which", "who", "whom", "not", "no", "nor", "if",
    "then", "than", "so", "just", "about", "into", "through", "during",
    "before", "after", "above", "below", "between", "same", "such",
];

/// Tokenize raw text into a list of stemmed, lowercased terms.
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .filter(|w| !STOP_WORDS.contains(w))
        .map(|w| stem(w))
        .collect()
}

// ── Simplified Porter Stemmer ────────────────────────────────────────
// We implement the core steps of the Porter algorithm by hand.
// It's not perfect (the real one has ~60 rules) but it's solid enough
// to collapse "running" → "run", "flies" → "fli", "happily" → "happili", etc.

fn measure(s: &str) -> usize {
    // Count VC sequences (consonant-vowel transitions)
    let mut m = 0;
    let mut prev_vowel = false;
    for c in s.chars() {
        let is_v = is_vowel_char(c);
        if !is_v && prev_vowel {
            m += 1;
        }
        prev_vowel = is_v;
    }
    m
}

fn is_vowel_char(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

fn has_vowel(s: &str) -> bool {
    s.chars().any(is_vowel_char)
}

fn ends_double_consonant(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 2 { return false; }
    let last = bytes[bytes.len() - 1];
    let prev = bytes[bytes.len() - 2];
    last == prev && !is_vowel_char(last as char)
}

fn ends_cvc(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 { return false; }
    let (c1, v, c2) = (
        chars[chars.len() - 3],
        chars[chars.len() - 2],
        chars[chars.len() - 1],
    );
    !is_vowel_char(c1) && is_vowel_char(v) && !is_vowel_char(c2)
        && !matches!(c2, 'w' | 'x' | 'y')
}

fn replace_suffix<'a>(word: &'a str, suffix: &str, replacement: &str) -> Option<String> {
    if word.ends_with(suffix) {
        let stem = &word[..word.len() - suffix.len()];
        Some(format!("{}{}", stem, replacement))
    } else {
        None
    }
}

pub fn stem(word: &str) -> String {
    if word.len() <= 2 {
        return word.to_string();
    }
    let mut w = word.to_string();

    // Step 1a: plurals
    if w.ends_with("sses") {
        w.truncate(w.len() - 2);
    } else if w.ends_with("ies") {
        w.truncate(w.len() - 2);
    } else if !w.ends_with("ss") && w.ends_with('s') {
        w.pop();
    }

    // Step 1b: -eed, -ed, -ing
    if w.ends_with("eed") {
        let stem_part = &w[..w.len() - 3];
        if measure(stem_part) > 0 {
            w.truncate(w.len() - 1); // eed → ee
        }
    } else if w.ends_with("ed") {
        let stem_part = &w[..w.len() - 2];
        if has_vowel(stem_part) {
            w.truncate(w.len() - 2);
            w = step1b_fixup(w);
        }
    } else if w.ends_with("ing") {
        let stem_part = &w[..w.len() - 3];
        if has_vowel(stem_part) {
            w.truncate(w.len() - 3);
            w = step1b_fixup(w);
        }
    }

    // Step 1c: y → i
    if w.ends_with('y') {
        let stem_part = &w[..w.len() - 1];
        if has_vowel(stem_part) {
            w.pop();
            w.push('i');
        }
    }

    // Step 2: longer suffixes
    let step2_rules: &[(&str, &str)] = &[
        ("ational", "ate"), ("tional", "tion"), ("enci", "ence"),
        ("anci", "ance"), ("izer", "ize"), ("abli", "able"),
        ("alli", "al"), ("entli", "ent"), ("eli", "e"),
        ("ousli", "ous"), ("ization", "ize"), ("ation", "ate"),
        ("ator", "ate"), ("alism", "al"), ("iveness", "ive"),
        ("fulness", "ful"), ("ousness", "ous"), ("aliti", "al"),
        ("iviti", "ive"), ("biliti", "ble"),
    ];
    for &(suffix, replacement) in step2_rules {
        if let Some(replaced) = replace_suffix(&w, suffix, replacement) {
            let stem_part = &w[..w.len() - suffix.len()];
            if measure(stem_part) > 0 {
                w = replaced;
            }
            break;
        }
    }

    // Step 3
    let step3_rules: &[(&str, &str)] = &[
        ("icate", "ic"), ("ative", ""), ("alize", "al"),
        ("iciti", "ic"), ("ical", "ic"), ("ful", ""), ("ness", ""),
    ];
    for &(suffix, replacement) in step3_rules {
        if let Some(replaced) = replace_suffix(&w, suffix, replacement) {
            let stem_part = &w[..w.len() - suffix.len()];
            if measure(stem_part) > 0 {
                w = replaced;
            }
            break;
        }
    }

    // Step 4: remove long suffixes if m > 1
    let step4_suffixes: &[&str] = &[
        "al", "ance", "ence", "er", "ic", "able", "ible", "ant",
        "ement", "ment", "ent", "ion", "ou", "ism", "ate", "iti",
        "ous", "ive", "ize",
    ];
    for &suffix in step4_suffixes {
        if w.ends_with(suffix) {
            let stem_part = &w[..w.len() - suffix.len()];
            if measure(stem_part) > 1 {
                if suffix == "ion" {
                    // special: stem must end in s or t
                    if stem_part.ends_with('s') || stem_part.ends_with('t') {
                        w.truncate(w.len() - suffix.len());
                    }
                } else {
                    w.truncate(w.len() - suffix.len());
                }
            }
            break;
        }
    }

    // Step 5a: remove trailing 'e'
    if w.ends_with('e') {
        let stem_part = &w[..w.len() - 1];
        let m = measure(stem_part);
        if m > 1 || (m == 1 && !ends_cvc(stem_part)) {
            w.pop();
        }
    }

    // Step 5b: ll → l
    if w.ends_with("ll") && measure(&w[..w.len() - 1]) > 1 {
        w.pop();
    }

    w
}

fn step1b_fixup(mut w: String) -> String {
    if w.ends_with("at") || w.ends_with("bl") || w.ends_with("iz") {
        w.push('e');
    } else if ends_double_consonant(&w) && !w.ends_with('l') && !w.ends_with('s') && !w.ends_with('z') {
        w.pop();
    } else if measure(&w) == 1 && ends_cvc(&w) {
        w.push('e');
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_stemming() {
        assert_eq!(stem("running"), "run");
        assert_eq!(stem("cats"), "cat");
        assert_eq!(stem("connected"), "connect");
    }

    #[test]
    fn test_tokenize_removes_stop_words() {
        let tokens = tokenize("the quick brown fox jumps over the lazy dog");
        assert!(!tokens.contains(&"the".to_string()));
        assert!(tokens.contains(&stem("quick")));
    }
}
