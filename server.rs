/// Tiny HTTP search UI. One page, one form, one results endpoint.
/// Uses the same ranking pipeline as the CLI: BM25 → rerank with title/URL
/// boosts and PageRank, with auto spell-correction on zero results.

use std::time::Instant;
use tiny_http::{Header, Method, Response, Server};

use crate::bktree::SpellCorrector;
use crate::index::InvertedIndex;
use crate::ranking::{self, BM25Config, SearchResult};
use crate::store::DocumentStore;

pub fn serve(
    port: u16,
    index: &InvertedIndex,
    store: &DocumentStore,
    pageranks: &[f32],
    spell: &SpellCorrector,
) -> std::io::Result<()> {
    let addr = format!("0.0.0.0:{}", port);
    let server = Server::http(&addr)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    println!("🌐 Serving on http://localhost:{}/", port);

    let cfg = BM25Config::default();

    for request in server.incoming_requests() {
        let url = request.url().to_string();
        let (path, query_str) = match url.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (url, String::new()),
        };

        let (status, ctype, body) = match (request.method(), path.as_str()) {
            (Method::Get, "/") => {
                let q = extract_query(&query_str);
                let html = if q.is_empty() {
                    home_page()
                } else {
                    results_page(&q, index, store, pageranks, spell, &cfg)
                };
                (200, "text/html; charset=utf-8", html)
            }
            _ => (404, "text/plain; charset=utf-8", "Not Found".to_string()),
        };

        let response = Response::from_string(body)
            .with_status_code(status)
            .with_header(
                Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes()).unwrap(),
            );
        let _ = request.respond(response);
    }
    Ok(())
}

fn extract_query(query_str: &str) -> String {
    url::form_urlencoded::parse(query_str.as_bytes())
        .find(|(k, _)| k == "q")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

fn run_search(
    query: &str,
    index: &InvertedIndex,
    store: &DocumentStore,
    pageranks: &[f32],
    spell: &SpellCorrector,
    cfg: &BM25Config,
) -> (Vec<SearchResult>, Option<String>) {
    let mut results = ranking::search(index, query, cfg, 50);
    ranking::rerank_results(&mut results, store, pageranks, query, 0.35);
    results.truncate(20);

    // Always check for a spell correction — show "Did you mean" even when
    // there are results (just like Google).  If zero results, swap in the
    // corrected set automatically.
    let mut suggestion = None;
    if let Some(c) = spell.correct_query(query) {
        if c.corrected != query {
            suggestion = Some(c.corrected.clone());
            if results.is_empty() {
                let mut corrected = ranking::search(index, &c.corrected, cfg, 50);
                ranking::rerank_results(&mut corrected, store, pageranks, &c.corrected, 0.35);
                corrected.truncate(20);
                results = corrected;
            }
        }
    }
    (results, suggestion)
}

fn results_page(
    query: &str,
    index: &InvertedIndex,
    store: &DocumentStore,
    pageranks: &[f32],
    spell: &SpellCorrector,
    cfg: &BM25Config,
) -> String {
    let start = Instant::now();
    let (results, suggestion) = run_search(query, index, store, pageranks, spell, cfg);
    let elapsed = start.elapsed();

    let mut out = String::new();
    out.push_str(&header(query));

    out.push_str(&format!(
        r#"<div class="meta">About {} results ({:.3} seconds)</div>"#,
        results.len(),
        elapsed.as_secs_f64(),
    ));

    if let Some(ref c) = suggestion {
        if results.is_empty() {
            out.push_str(&format!(
                r#"<div class="suggest">Showing results for <a href="/?q={}"><b>{}</b></a></div>"#,
                esc(c), esc(c),
            ));
        } else {
            out.push_str(&format!(
                r#"<div class="suggest">Did you mean: <a href="/?q={}"><b><i>{}</i></b></a>?</div>"#,
                esc(c), esc(c),
            ));
        }
    }

    if results.is_empty() {
        out.push_str(r#"<div class="empty">No results found.</div>"#);
    } else {
        out.push_str(r#"<div class="results">"#);
        for r in &results {
            if let Some(doc) = store.get(r.doc_id) {
                let snippet = store.snippet(r.doc_id, &r.matched_terms, 14);
                let is_url = doc.path.starts_with("http://") || doc.path.starts_with("https://");
                let link = if is_url {
                    format!(
                        r#"<a class="title" href="{}" target="_blank" rel="noopener">{}</a>"#,
                        esc(&doc.path),
                        esc(&doc.title),
                    )
                } else {
                    format!(r#"<span class="title">{}</span>"#, esc(&doc.title))
                };
                out.push_str(&format!(
                    r#"<div class="result"><div class="url">{}</div>{}<div class="snippet">{}</div><div class="score">score {:.3}</div></div>"#,
                    esc(&doc.path),
                    link,
                    esc(&snippet),
                    r.score,
                ));
            }
        }
        out.push_str("</div>");
    }

    out.push_str(FOOTER);
    out
}

fn home_page() -> String {
    let mut out = String::new();
    out.push_str(HEAD);
    out.push_str(
        r#"<div class="landing">
  <h1>Zehnyasearch</h1>
  <form action="/" method="get" class="bigform">
    <input name="q" type="text" autofocus placeholder="Search the crawled web...">
    <button>Search</button>
  </form>
</div>"#,
    );
    out.push_str(FOOTER);
    out
}

fn header(query: &str) -> String {
    let mut out = String::new();
    out.push_str(HEAD);
    out.push_str(&format!(
        r#"<header>
  <a class="brand" href="/">Zehnyasearch</a>
  <form action="/" method="get" class="searchbar">
    <input name="q" type="text" value="{}">
    <button>Search</button>
  </form>
</header>"#,
        esc(query),
    ));
    out
}

const HEAD: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Zehnyasearch</title>
<style>
  *{box-sizing:border-box}
  body{font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif;margin:0;color:#202124;background:#fff}
  a{color:#1a0dab;text-decoration:none}
  a:hover{text-decoration:underline}
  header{display:flex;align-items:center;gap:24px;padding:16px 28px;border-bottom:1px solid #ebebeb}
  .brand{font-size:22px;font-weight:600;color:#4285f4;text-decoration:none}
  .searchbar{flex:1;display:flex;gap:8px;max-width:640px}
  .searchbar input{flex:1;padding:10px 16px;font-size:15px;border:1px solid #dfe1e5;border-radius:24px;outline:none}
  .searchbar input:focus{border-color:#4285f4;box-shadow:0 1px 6px rgba(32,33,36,0.16)}
  .searchbar button,.bigform button{padding:10px 20px;font-size:14px;border:0;background:#4285f4;color:#fff;border-radius:24px;cursor:pointer}
  .meta{padding:12px 28px;color:#70757a;font-size:13px}
  .suggest{padding:8px 28px 8px;font-size:16px;color:#70757a}
  .suggest a{color:#1a0dab;font-size:18px}
  .suggest a:hover{text-decoration:underline}
  .empty{padding:40px 28px;color:#70757a}
  .results{padding:0 28px 40px;max-width:720px}
  .result{margin:0 0 28px}
  .result .url{color:#202124;font-size:12px;margin-bottom:2px;word-break:break-all}
  .result .title{font-size:20px;line-height:1.3;color:#1a0dab}
  .result .snippet{color:#4d5156;font-size:14px;line-height:1.58;margin-top:4px}
  .result .score{color:#9aa0a6;font-size:11px;margin-top:4px;font-family:ui-monospace,monospace}
  .landing{max-width:600px;margin:120px auto;padding:0 24px;text-align:center}
  .landing h1{font-weight:300;font-size:64px;color:#4285f4;margin:0 0 32px}
  .bigform{display:flex;gap:8px}
  .bigform input{flex:1;padding:14px 20px;font-size:16px;border:1px solid #dfe1e5;border-radius:28px;outline:none}
  .bigform input:focus{border-color:#4285f4;box-shadow:0 1px 6px rgba(32,33,36,0.16)}
</style>
</head>
<body>"#;

const FOOTER: &str = r#"</body></html>"#;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
