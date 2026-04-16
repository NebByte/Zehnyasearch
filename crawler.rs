/// Breadth-first web crawler. Feeds pages straight into the existing
/// InvertedIndex + DocumentStore so crawled pages are searchable by every
/// mode (BM25, phrase, boolean, fuzzy, spell) with no extra work.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use scraper::{Html, Selector};
use url::Url;

use crate::index::InvertedIndex;
use crate::store::DocumentStore;

pub struct CrawlConfig {
    pub max_pages: usize,
    pub max_depth: u32,
    pub delay_ms: u64,
    pub same_domain_only: bool,
    pub user_agent: String,
    pub timeout_secs: u64,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            max_pages: 50,
            max_depth: 2,
            delay_ms: 500,
            same_domain_only: true,
            user_agent: "ZehnyasearchBot/0.1".to_string(),
            timeout_secs: 10,
        }
    }
}

pub struct CrawlStats {
    pub pages_fetched: usize,
    pub pages_indexed: usize,
    pub pages_skipped: usize,
    pub bytes_fetched: u64,
    pub elapsed: Duration,
    /// Outbound adjacency for *newly crawled* pages only (append to existing graph).
    pub link_graph: Vec<Vec<u32>>,
    /// URL → doc_id map for all pages crawled in this run (merge with existing).
    pub url_map: HashMap<String, u32>,
}

struct RobotRules {
    disallowed: Vec<String>,
}

impl RobotRules {
    fn allow_all() -> Self { Self { disallowed: Vec::new() } }

    fn parse(body: &str) -> Self {
        let mut disallowed = Vec::new();
        let mut applies = false;
        for line in body.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() { continue; }
            let (k, v) = match line.split_once(':') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().to_string()),
                None => continue,
            };
            match k.as_str() {
                "user-agent" => applies = v == "*",
                "disallow" if applies && !v.is_empty() => disallowed.push(v),
                _ => {}
            }
        }
        Self { disallowed }
    }

    fn allowed(&self, path: &str) -> bool {
        !self.disallowed.iter().any(|p| path.starts_with(p.as_str()))
    }
}

pub fn crawl(
    seeds: &[String],
    config: &CrawlConfig,
    inv_index: &mut InvertedIndex,
    doc_store: &mut DocumentStore,
    existing_urls: Option<&HashMap<String, u32>>,
) -> CrawlStats {
    let started = Instant::now();

    let mut frontier: VecDeque<(String, u32)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut last_hit: HashMap<String, Instant> = HashMap::new();
    let mut robots_cache: HashMap<String, RobotRules> = HashMap::new();
    let mut seed_hosts: HashSet<String> = HashSet::new();

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(config.timeout_secs))
        .user_agent(&config.user_agent)
        .build();

    // Seed url_to_id from existing corpus for link resolution.
    // DO NOT add to `visited` — we still need to visit existing pages
    // to discover their outbound links and expand the frontier.
    let mut url_to_id: HashMap<String, u32> = HashMap::new();
    if let Some(existing) = existing_urls {
        for (url, id) in existing {
            url_to_id.insert(url.clone(), *id);
        }
    }

    for s in seeds {
        if let Ok(u) = Url::parse(s) {
            if let Some(h) = u.host_str() {
                seed_hosts.insert(h.to_string());
            }
            let norm = normalize(&u);
            if visited.insert(norm.clone()) {
                frontier.push_back((norm, 0));
            }
        }
    }

    let mut stats = CrawlStats {
        pages_fetched: 0,
        pages_indexed: 0,
        pages_skipped: 0,
        bytes_fetched: 0,
        elapsed: Duration::ZERO,
        link_graph: Vec::new(),
        url_map: HashMap::new(),
    };

    let mut outbound_urls: Vec<Vec<String>> = Vec::new();

    while let Some((url_str, depth)) = frontier.pop_front() {
        if stats.pages_indexed >= config.max_pages { break; }

        let url = match Url::parse(&url_str) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let host = match url.host_str() {
            Some(h) => h.to_string(),
            None => continue,
        };

        if config.same_domain_only && !seed_hosts.contains(&host) {
            stats.pages_skipped += 1;
            continue;
        }

        if !robots_cache.contains_key(&host) {
            let robots_url = format!("{}://{}/robots.txt", url.scheme(), host);
            let rules = fetch_robots(&agent, &robots_url);
            robots_cache.insert(host.clone(), rules);
            last_hit.insert(host.clone(), Instant::now());
        }
        if !robots_cache.get(&host).unwrap().allowed(url.path()) {
            stats.pages_skipped += 1;
            continue;
        }

        if let Some(last) = last_hit.get(&host) {
            let min = Duration::from_millis(config.delay_ms);
            let e = last.elapsed();
            if e < min {
                std::thread::sleep(min - e);
            }
        }

        let response = match agent.get(url.as_str()).call() {
            Ok(r) => r,
            Err(_) => {
                stats.pages_skipped += 1;
                last_hit.insert(host, Instant::now());
                continue;
            }
        };
        last_hit.insert(host.clone(), Instant::now());
        stats.pages_fetched += 1;

        let ct = response.header("content-type").unwrap_or("").to_ascii_lowercase();
        if !ct.contains("text/html") && !ct.contains("text/plain") {
            stats.pages_skipped += 1;
            continue;
        }

        let body = match response.into_string() {
            Ok(b) => b,
            Err(_) => { stats.pages_skipped += 1; continue; }
        };
        stats.bytes_fetched += body.len() as u64;

        let (title, text, links) = extract(&body, &url);

        let already_indexed = url_to_id.contains_key(&url_str);

        if !already_indexed {
            if text.trim().is_empty() {
                stats.pages_skipped += 1;
                // Still expand links below even for empty pages
            } else {
                let doc_id = doc_store.add(url_str.clone(), title, text.clone());
                inv_index.add_document(doc_id, &text);
                url_to_id.insert(url_str.clone(), doc_id);
                outbound_urls.push(links.iter().map(normalize).collect());
                stats.pages_indexed += 1;

                if stats.pages_indexed % 10 == 0 {
                    eprint!("  crawled {} pages...\r", stats.pages_indexed);
                }
            }
        }
        // Always expand links to discover new pages at deeper depths.
        if depth < config.max_depth {
            for link in links {
                let norm = normalize(&link);
                if visited.insert(norm.clone()) {
                    frontier.push_back((norm, depth + 1));
                }
            }
        }
    }

    // Resolve outbound URLs → doc_ids for indexed pages only.
    let mut link_graph: Vec<Vec<u32>> = Vec::with_capacity(outbound_urls.len());
    for dsts in outbound_urls {
        let mut resolved: Vec<u32> = dsts
            .iter()
            .filter_map(|u| url_to_id.get(u).copied())
            .collect();
        resolved.sort_unstable();
        resolved.dedup();
        link_graph.push(resolved);
    }
    stats.link_graph = link_graph;
    stats.url_map = url_to_id;

    stats.elapsed = started.elapsed();
    stats
}

fn fetch_robots(agent: &ureq::Agent, url: &str) -> RobotRules {
    match agent.get(url).call() {
        Ok(r) => match r.into_string() {
            Ok(body) => RobotRules::parse(&body),
            Err(_) => RobotRules::allow_all(),
        },
        Err(_) => RobotRules::allow_all(),
    }
}

fn normalize(u: &Url) -> String {
    let mut u = u.clone();
    u.set_fragment(None);
    u.to_string()
}

fn extract(html: &str, base: &Url) -> (String, String, Vec<Url>) {
    let doc = Html::parse_document(html);

    let title = Selector::parse("title").unwrap();
    let title_text = doc
        .select(&title)
        .next()
        .map(|n| n.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| base.to_string());

    let mut text = String::new();
    let mut stack = Vec::new();
    stack.push(doc.tree.root());
    while let Some(node) = stack.pop() {
        let v = node.value();
        if let Some(el) = v.as_element() {
            let name = el.name();
            if matches!(name, "script" | "style" | "noscript" | "template") {
                continue;
            }
        }
        if let Some(t) = v.as_text() {
            let s = t.trim();
            if !s.is_empty() {
                text.push_str(s);
                text.push(' ');
            }
            continue;
        }
        for child in node.children() {
            stack.push(child);
        }
    }

    let a = Selector::parse("a[href]").unwrap();
    let mut links = Vec::new();
    for el in doc.select(&a) {
        if let Some(href) = el.value().attr("href") {
            if let Ok(resolved) = base.join(href) {
                if matches!(resolved.scheme(), "http" | "https") {
                    links.push(resolved);
                }
            }
        }
    }

    (title_text, text, links)
}
