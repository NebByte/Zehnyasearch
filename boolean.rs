/// Boolean query engine: parses and evaluates queries like
///   "TCP AND socket NOT UDP"
///   "consensus OR (leader AND election)"
///   "memory AND (heap OR stack) NOT garbage"
///
/// Hand-rolled recursive descent parser + set operations on postings.

use std::collections::HashSet;
use crate::index::InvertedIndex;
use crate::ranking::{self, BM25Config, SearchResult};
use crate::tokenizer;

// ── AST ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum BoolExpr {
    Term(String),
    And(Box<BoolExpr>, Box<BoolExpr>),
    Or(Box<BoolExpr>, Box<BoolExpr>),
    Not(Box<BoolExpr>),
}

// ── Tokenizer (for the query language, not documents) ────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    And,
    Or,
    Not,
    LParen,
    RParen,
}

fn tokenize_query(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '(' {
            tokens.push(Token::LParen);
            chars.next();
        } else if c == ')' {
            tokens.push(Token::RParen);
            chars.next();
        } else if c == '-' {
            // -term means NOT term
            chars.next();
            let word = consume_word(&mut chars);
            if !word.is_empty() {
                tokens.push(Token::Not);
                tokens.push(Token::Word(word));
            }
        } else {
            let word = consume_word(&mut chars);
            match word.to_uppercase().as_str() {
                "AND" => tokens.push(Token::And),
                "OR" => tokens.push(Token::Or),
                "NOT" => tokens.push(Token::Not),
                _ => {
                    if !word.is_empty() {
                        tokens.push(Token::Word(word));
                    }
                }
            }
        }
    }

    tokens
}

fn consume_word(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut word = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            word.push(c);
            chars.next();
        } else {
            break;
        }
    }
    word
}

// ── Recursive Descent Parser ─────────────────────────────────────────
// Grammar:
//   expr     → or_expr
//   or_expr  → and_expr ("OR" and_expr)*
//   and_expr → not_expr ("AND" not_expr)*
//              (also implicit AND: "tcp socket" = "tcp AND socket")
//   not_expr → "NOT" atom | atom
//   atom     → WORD | "(" expr ")"

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    fn parse(&mut self) -> Option<BoolExpr> {
        let expr = self.or_expr()?;
        Some(expr)
    }

    fn or_expr(&mut self) -> Option<BoolExpr> {
        let mut left = self.and_expr()?;
        while self.peek() == Some(&Token::Or) {
            self.advance(); // consume OR
            let right = self.and_expr()?;
            left = BoolExpr::Or(Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn and_expr(&mut self) -> Option<BoolExpr> {
        let mut left = self.not_expr()?;
        loop {
            match self.peek() {
                Some(Token::And) => {
                    self.advance(); // consume AND
                    let right = self.not_expr()?;
                    left = BoolExpr::And(Box::new(left), Box::new(right));
                }
                // Implicit AND: two terms next to each other without operator
                Some(Token::Word(_)) | Some(Token::Not) | Some(Token::LParen) => {
                    let right = self.not_expr()?;
                    left = BoolExpr::And(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Some(left)
    }

    fn not_expr(&mut self) -> Option<BoolExpr> {
        if self.peek() == Some(&Token::Not) {
            self.advance(); // consume NOT
            let expr = self.atom()?;
            return Some(BoolExpr::Not(Box::new(expr)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<BoolExpr> {
        match self.peek() {
            Some(Token::Word(_)) => {
                if let Some(Token::Word(w)) = self.advance() {
                    // Stem the word to match our index
                    let stemmed = tokenizer::stem(&w.to_lowercase());
                    Some(BoolExpr::Term(stemmed))
                } else {
                    None
                }
            }
            Some(Token::LParen) => {
                self.advance(); // consume (
                let expr = self.parse()?;
                if self.peek() == Some(&Token::RParen) {
                    self.advance(); // consume )
                }
                Some(expr)
            }
            _ => None,
        }
    }
}

/// Parse a boolean query string into an AST.
pub fn parse(query: &str) -> Option<BoolExpr> {
    let tokens = tokenize_query(query);
    if tokens.is_empty() {
        return None;
    }
    let mut parser = Parser::new(tokens);
    parser.parse()
}

// ── Evaluation ───────────────────────────────────────────────────────
// Evaluate the AST against the inverted index to get a set of matching doc IDs.

pub fn evaluate(expr: &BoolExpr, index: &InvertedIndex) -> HashSet<u32> {
    match expr {
        BoolExpr::Term(term) => {
            index
                .get_postings(term)
                .iter()
                .map(|p| p.doc_id)
                .collect()
        }
        BoolExpr::And(left, right) => {
            let l = evaluate(left, index);
            let r = evaluate(right, index);
            l.intersection(&r).copied().collect()
        }
        BoolExpr::Or(left, right) => {
            let l = evaluate(left, index);
            let r = evaluate(right, index);
            l.union(&r).copied().collect()
        }
        BoolExpr::Not(inner) => {
            // NOT returns all documents MINUS the inner set
            let all_docs: HashSet<u32> = index.doc_lengths.keys().copied().collect();
            let exclude = evaluate(inner, index);
            all_docs.difference(&exclude).copied().collect()
        }
    }
}

/// Full boolean search: parse the query, evaluate to get matching doc IDs,
/// then BM25-rank the matching set.
pub fn boolean_search(
    index: &InvertedIndex,
    query: &str,
    config: &BM25Config,
    max_results: usize,
) -> BooleanSearchResult {
    let ast = match parse(query) {
        Some(a) => a,
        None => {
            return BooleanSearchResult {
                results: vec![],
                ast_debug: "Failed to parse query".to_string(),
                matching_docs: 0,
            };
        }
    };

    let ast_debug = format!("{:?}", ast);

    // Get the set of matching documents
    let matching_doc_ids = evaluate(&ast, index);
    let matching_docs = matching_doc_ids.len();

    // Extract the terms from the AST for BM25 scoring
    let terms = extract_terms(&ast);
    let scoring_query = terms.join(" ");

    // Score with BM25, but only keep docs that match the boolean filter
    let all_results = ranking::search(index, &scoring_query, config, max_results * 10);
    let mut filtered: Vec<SearchResult> = all_results
        .into_iter()
        .filter(|r| matching_doc_ids.contains(&r.doc_id))
        .collect();

    filtered.truncate(max_results);

    BooleanSearchResult {
        results: filtered,
        ast_debug,
        matching_docs,
    }
}

pub struct BooleanSearchResult {
    pub results: Vec<SearchResult>,
    pub ast_debug: String,
    pub matching_docs: usize,
}

/// Extract all positive terms from the AST (for BM25 scoring).
fn extract_terms(expr: &BoolExpr) -> Vec<String> {
    match expr {
        BoolExpr::Term(t) => vec![t.clone()],
        BoolExpr::And(l, r) | BoolExpr::Or(l, r) => {
            let mut terms = extract_terms(l);
            terms.extend(extract_terms(r));
            terms
        }
        BoolExpr::Not(_) => vec![], // Don't score on negated terms
    }
}

/// Pretty-print a boolean expression for display.
pub fn format_expr(expr: &BoolExpr) -> String {
    match expr {
        BoolExpr::Term(t) => t.clone(),
        BoolExpr::And(l, r) => format!("({} AND {})", format_expr(l), format_expr(r)),
        BoolExpr::Or(l, r) => format!("({} OR {})", format_expr(l), format_expr(r)),
        BoolExpr::Not(inner) => format!("NOT {}", format_expr(inner)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_and() {
        let expr = parse("tcp AND socket").unwrap();
        assert!(matches!(expr, BoolExpr::And(_, _)));
    }

    #[test]
    fn test_parse_implicit_and() {
        let expr = parse("tcp socket").unwrap();
        // Two adjacent words = implicit AND
        assert!(matches!(expr, BoolExpr::And(_, _)));
    }

    #[test]
    fn test_parse_not() {
        let expr = parse("tcp NOT udp").unwrap();
        if let BoolExpr::And(_, right) = expr {
            assert!(matches!(*right, BoolExpr::Not(_)));
        }
    }

    #[test]
    fn test_parse_parens() {
        let expr = parse("(tcp OR udp) AND socket").unwrap();
        assert!(matches!(expr, BoolExpr::And(_, _)));
    }
}
