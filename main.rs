mod tokenizer;
mod index;
mod ranking;
mod store;
mod fuzzy;
mod boolean;
mod compression;
mod bktree;
mod crawler;
mod pagerank;
mod corpus;
mod server;

use std::io::{self, Write, BufRead};
use std::path::{Path, PathBuf};
use std::time::Instant;

struct CliConfig {
    target: Option<String>,
    load_path: Option<PathBuf>,
    save_path: Option<PathBuf>,
    serve: bool,
    port: u16,
    bench: bool,
    crawl: crawler::CrawlConfig,
}

fn parse_args(args: &[String]) -> CliConfig {
    let mut cfg = CliConfig {
        target: None,
        load_path: None,
        save_path: None,
        serve: false,
        port: 8080,
        bench: false,
        crawl: crawler::CrawlConfig::default(),
    };

    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--load" if i + 1 < args.len() => { cfg.load_path = Some(PathBuf::from(&args[i+1])); i += 2; }
            "--save" if i + 1 < args.len() => { cfg.save_path = Some(PathBuf::from(&args[i+1])); i += 2; }
            "--serve" => { cfg.serve = true; i += 1; }
            "--port" if i + 1 < args.len() => {
                if let Ok(p) = args[i+1].parse() { cfg.port = p; }
                i += 2;
            }
            "--bench" => { cfg.bench = true; i += 1; }
            "--max-pages" if i + 1 < args.len() => {
                if let Ok(n) = args[i+1].parse() { cfg.crawl.max_pages = n; }
                i += 2;
            }
            "--depth" if i + 1 < args.len() => {
                if let Ok(n) = args[i+1].parse() { cfg.crawl.max_depth = n; }
                i += 2;
            }
            "--delay" if i + 1 < args.len() => {
                if let Ok(n) = args[i+1].parse() { cfg.crawl.delay_ms = n; }
                i += 2;
            }
            "--cross-domain" => { cfg.crawl.same_domain_only = false; i += 1; }
            s if !s.starts_with("--") => { positional.push(s.to_string()); i += 1; }
            _ => { i += 1; }
        }
    }

    cfg.target = positional.into_iter().next();
    cfg
}

fn usage_and_exit() -> ! {
    eprintln!("Usage:");
    eprintln!("  search-engine <directory>                       index local files");
    eprintln!("  search-engine <url>                             crawl & index the web");
    eprintln!("  search-engine --load corpus.bin                 reload a saved corpus");
    eprintln!("  search-engine --load corpus.bin <url>           incremental crawl");
    eprintln!();
    eprintln!("Modifiers:");
    eprintln!("  --save <path>         save the corpus to disk after indexing");
    eprintln!("  --load <path>         load existing corpus (combine with <url> to add more)");
    eprintln!("  --serve               start HTTP UI (default port 8080)");
    eprintln!("  --port <N>            override HTTP port");
    eprintln!("  --bench               run built-in benchmark instead of REPL");
    eprintln!();
    eprintln!("Crawl flags: --max-pages N  --depth N  --delay MS  --cross-domain");
    std::process::exit(1);
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() {
        usage_and_exit();
    }
    let cli = parse_args(&raw);

    // ── Load or create the corpus, optionally crawl more into it ──────
    let mut inv_index;
    let mut doc_store;
    let mut link_graph: Vec<Vec<u32>>;
    let mut url_map: std::collections::HashMap<String, u32>;
    let mut pageranks: Vec<f32>;

    if let Some(path) = cli.load_path.as_ref() {
        // Load existing corpus
        print!("📂 Loading corpus from {} ...", path.display());
        io::stdout().flush().unwrap();
        let start = Instant::now();
        match corpus::Corpus::load(path) {
            Ok(c) => {
                println!(" done ({:.2?})", start.elapsed());
                println!(
                    "   {} documents, {} PageRank nodes loaded.",
                    c.store.count(), c.pageranks.len(),
                );
                inv_index = c.index;
                doc_store = c.store;
                link_graph = c.link_graph;
                pageranks = c.pageranks;
                url_map = c.url_map;
            }
            Err(e) => {
                eprintln!("\n❌ Failed to load corpus: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        inv_index = index::InvertedIndex::new();
        doc_store = store::DocumentStore::new();
        link_graph = Vec::new();
        pageranks = Vec::new();
        url_map = std::collections::HashMap::new();
    }

    // If a target is given, index/crawl into the (possibly loaded) corpus.
    if let Some(target) = cli.target.as_deref() {
        let is_url = target.starts_with("http://") || target.starts_with("https://");
        if is_url {
            let existing_count = doc_store.count();
            println!(
                "⚡ Crawling '{}' (max {} pages, depth {}, delay {}ms, {})...",
                target,
                cli.crawl.max_pages,
                cli.crawl.max_depth,
                cli.crawl.delay_ms,
                if cli.crawl.same_domain_only { "same-domain" } else { "cross-domain" },
            );
            let stats = crawler::crawl(
                &[target.to_string()],
                &cli.crawl,
                &mut inv_index,
                &mut doc_store,
                if url_map.is_empty() { None } else { Some(&url_map) },
            );

            println!();
            println!("── Crawl Stats ──────────────────────────────");
            println!("Pages fetched: {}", stats.pages_fetched);
            println!("Pages indexed: {} (total now: {})", stats.pages_indexed, doc_store.count());
            println!("Pages skipped: {}", stats.pages_skipped);
            println!("Bytes fetched: {}", format_bytes(stats.bytes_fetched));
            println!("Crawl time:    {:.2?}", stats.elapsed);
            if existing_count > 0 {
                println!("Previously:    {} docs already in corpus", existing_count);
            }

            // Merge new link graph + url_map
            link_graph.extend(stats.link_graph);
            for (u, id) in &stats.url_map {
                url_map.insert(u.clone(), *id);
            }

            // Recompute PageRank on full graph
            print!("📈 Computing PageRank...");
            io::stdout().flush().unwrap();
            let pr_start = Instant::now();
            pageranks = pagerank::compute(&link_graph, 0.85, 30);
            println!(" done ({} nodes in {:.2?})", pageranks.len(), pr_start.elapsed());
        } else {
            println!("⚡ Indexing '{}' ...", target);
            let mut file_count = 0u32;
            let mut byte_count = 0u64;
            index_directory(
                Path::new(target), &mut inv_index, &mut doc_store,
                &mut file_count, &mut byte_count,
            );
            println!("Indexed {} files ({}).", file_count, format_bytes(byte_count));
        }
    } else if cli.load_path.is_none() {
        usage_and_exit();
    }

    // Save if requested.
    if let Some(path) = cli.save_path.as_ref() {
        let corpus = corpus::Corpus {
            index: clone_index(&inv_index),
            store: clone_store(&doc_store),
            link_graph: link_graph.clone(),
            pageranks: pageranks.clone(),
            url_map: url_map.clone(),
        };
        match corpus.save(path) {
            Ok(bytes) => println!("💾 Saved corpus → {} ({})", path.display(), format_bytes(bytes)),
            Err(e) => eprintln!("⚠️  Save failed: {}", e),
        }
    }

    // Build BK-tree for spell correction.
    print!("🌳 Building BK-Tree for spell correction...");
    io::stdout().flush().unwrap();
    let bk_start = Instant::now();
    let vocabulary: Vec<String> = inv_index.map.keys().cloned().collect();
    let spell = bktree::SpellCorrector::new(&vocabulary);
    println!(" done ({} terms in {:.2?})", vocabulary.len(), bk_start.elapsed());

    println!();
    println!("── Index Stats ──────────────────────────────");
    println!("{}", inv_index.stats());
    let comp_stats = compression::compute_stats(&inv_index.map);
    println!(
        "Compression:  {} → {} ({:.1}% savings)",
        format_bytes(comp_stats.original_bytes as u64),
        format_bytes(comp_stats.compressed_bytes as u64),
        comp_stats.savings_percent,
    );
    if !pageranks.is_empty() {
        let max_pr = pageranks.iter().cloned().fold(0.0f32, f32::max);
        let avg_pr: f32 = pageranks.iter().sum::<f32>() / pageranks.len() as f32;
        println!("PageRank:     max {:.4}, avg {:.4} over {} nodes", max_pr, avg_pr, pageranks.len());
    }
    println!("─────────────────────────────────────────────\n");

    if cli.bench {
        run_benchmark(&inv_index, &doc_store, &spell);
        return;
    }

    if cli.serve {
        if let Err(e) = server::serve(cli.port, &inv_index, &doc_store, &pageranks, &spell) {
            eprintln!("Server error: {}", e);
        }
        return;
    }

    run_repl(&inv_index, &doc_store, &pageranks, &spell);
}

// Bincode consumes by value on save; rather than wrestle with ownership in
// main, we snapshot the struct with a deep clone helper here.
fn clone_index(src: &index::InvertedIndex) -> index::InvertedIndex {
    let bytes = bincode::serialize(src).expect("serialize index");
    bincode::deserialize(&bytes).expect("deserialize index")
}
fn clone_store(src: &store::DocumentStore) -> store::DocumentStore {
    let bytes = bincode::serialize(src).expect("serialize store");
    bincode::deserialize(&bytes).expect("deserialize store")
}

// ── REPL ─────────────────────────────────────────────────────────────

fn run_repl(
    inv_index: &index::InvertedIndex,
    doc_store: &store::DocumentStore,
    pageranks: &[f32],
    spell: &bktree::SpellCorrector,
) {
    print_help();
    let stdin = io::stdin();
    let cfg = ranking::BM25Config::default();
    let alpha = 0.35;

    loop {
        print!("\x1b[36msearch>\x1b[0m ");
        io::stdout().flush().unwrap();

        let mut query = String::new();
        if stdin.lock().read_line(&mut query).unwrap() == 0 {
            break;
        }
        let query = query.trim();
        if query.is_empty() { continue; }

        match query {
            "quit" | "exit" | "q" => break,
            "help" | "?" => { print_help(); continue; }
            "stats" => {
                println!("{}", inv_index.stats());
                let cs = compression::compute_stats(&inv_index.map);
                println!(
                    "Compression: {} → {} ({:.1}% savings)",
                    format_bytes(cs.original_bytes as u64),
                    format_bytes(cs.compressed_bytes as u64),
                    cs.savings_percent,
                );
                continue;
            }
            _ => {}
        }

        if let Some(phrase) = query.strip_prefix("phrase:") {
            let start = Instant::now();
            let mut results = ranking::phrase_search(inv_index, phrase.trim(), &cfg, 50);
            ranking::rerank_results(&mut results, doc_store, pageranks, phrase.trim(), alpha);
            results.truncate(10);
            print_results(&results, doc_store, start.elapsed(), "phrase");
        } else if let Some(bq) = query.strip_prefix("bool:") {
            let start = Instant::now();
            let mut r = boolean::boolean_search(inv_index, bq.trim(), &cfg, 50);
            ranking::rerank_results(&mut r.results, doc_store, pageranks, bq.trim(), alpha);
            r.results.truncate(10);
            let elapsed = start.elapsed();
            if let Some(ast) = boolean::parse(bq.trim()) {
                println!("  \x1b[90mParsed: {}\x1b[0m", boolean::format_expr(&ast));
            }
            println!("  \x1b[90m{} docs matched boolean filter\x1b[0m", r.matching_docs);
            print_results(&r.results, doc_store, elapsed, "boolean");
        } else if let Some(fq) = query.strip_prefix("fuzzy:") {
            let start = Instant::now();
            let mut r = fuzzy::fuzzy_search(inv_index, fq.trim(), &cfg, 50, 2);
            ranking::rerank_results(&mut r.results, doc_store, pageranks, fq.trim(), alpha);
            r.results.truncate(10);
            let elapsed = start.elapsed();
            if !r.corrections.is_empty() {
                let corrections: Vec<String> = r.corrections.iter()
                    .map(|(o, c, d)| format!("{} → {} (dist {})", o, c, d))
                    .collect();
                println!("  \x1b[33m📝 Fuzzy expansions: {}\x1b[0m", corrections.join(", "));
            }
            print_results(&r.results, doc_store, elapsed, "fuzzy");
        } else if let Some(sq) = query.strip_prefix("spell:") {
            let start = Instant::now();
            let sq = sq.trim();
            let correction = spell.correct_query(sq);
            let elapsed = start.elapsed();
            match correction {
                Some(c) => {
                    println!("  \x1b[33m💡 {}\x1b[0m  ({:.3?})", c, elapsed);
                    let ss = Instant::now();
                    let mut results = ranking::search(inv_index, &c.corrected, &cfg, 50);
                    ranking::rerank_results(&mut results, doc_store, pageranks, &c.corrected, alpha);
                    results.truncate(10);
                    print_results(&results, doc_store, ss.elapsed(), "spell-corrected");
                }
                None => {
                    println!("  \x1b[32m✓ No corrections needed.\x1b[0m  ({:.3?})", elapsed);
                    let ss = Instant::now();
                    let mut results = ranking::search(inv_index, sq, &cfg, 50);
                    ranking::rerank_results(&mut results, doc_store, pageranks, sq, alpha);
                    results.truncate(10);
                    print_results(&results, doc_store, ss.elapsed(), "bm25");
                }
            }
        } else {
            let start = Instant::now();
            let mut results = ranking::search(inv_index, query, &cfg, 50);
            ranking::rerank_results(&mut results, doc_store, pageranks, query, alpha);
            results.truncate(10);
            let elapsed = start.elapsed();

            if results.is_empty() {
                if let Some(c) = spell.correct_query(query) {
                    println!("  \x1b[33m💡 {} Searching corrected query...\x1b[0m", c);
                    let ss = Instant::now();
                    let mut cr = ranking::search(inv_index, &c.corrected, &cfg, 50);
                    ranking::rerank_results(&mut cr, doc_store, pageranks, &c.corrected, alpha);
                    cr.truncate(10);
                    print_results(&cr, doc_store, ss.elapsed(), "spell-corrected");
                } else {
                    println!("  No results found.\n");
                }
            } else {
                print_results(&results, doc_store, elapsed, "bm25");
            }
        }
    }
}

fn print_help() {
    println!("🔍 Search engine ready. Commands:");
    println!("   <query>            BM25 + rerank (title / URL / PageRank boosts)");
    println!("   phrase:<query>     Exact phrase matching");
    println!("   bool:<query>       Boolean: AND, OR, NOT, parentheses");
    println!("   fuzzy:<query>      Fuzzy search (edit distance ≤ 2)");
    println!("   spell:<query>      Spell check + search");
    println!("   stats              Show index & compression stats");
    println!("   help               This message");
    println!("   quit               Exit");
    println!();
}

fn print_results(
    results: &[ranking::SearchResult],
    doc_store: &store::DocumentStore,
    elapsed: std::time::Duration,
    mode: &str,
) {
    if results.is_empty() {
        println!("  No results found.\n");
        return;
    }

    println!(
        "\n  \x1b[32m{} results in {:.3?}\x1b[0m [{}]:\n",
        results.len(),
        elapsed,
        mode,
    );

    for (rank, result) in results.iter().enumerate() {
        let doc = doc_store.get(result.doc_id).unwrap();
        let snippet = doc_store.snippet(result.doc_id, &result.matched_terms, 8);
        println!(
            "  \x1b[1m{}.\x1b[0m [score: {:.3}] \x1b[1m{}\x1b[0m",
            rank + 1,
            result.score,
            doc.title,
        );
        println!("     \x1b[90m{}\x1b[0m", doc.path);
        if !snippet.is_empty() {
            println!("     {}\n", snippet);
        }
    }
}

// ── Directory indexing (unchanged) ───────────────────────────────────

fn index_directory(
    dir: &Path,
    inv_index: &mut index::InvertedIndex,
    doc_store: &mut store::DocumentStore,
    file_count: &mut u32,
    byte_count: &mut u64,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("  Warning: can't read {}: {}", dir.display(), e);
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.')
                || name == "node_modules"
                || name == "target"
                || name == "__pycache__"
            {
                continue;
            }
            index_directory(&path, inv_index, doc_store, file_count, byte_count);
        } else if is_indexable(&path) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let title = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let path_str = path.to_string_lossy().to_string();
                let len = content.len() as u64;

                let doc_id = doc_store.add(path_str, title, content.clone());
                inv_index.add_document(doc_id, &content);

                *file_count += 1;
                *byte_count += len;

                if *file_count % 1000 == 0 {
                    eprint!("  indexed {} files...\r", file_count);
                }
            }
        }
    }
}

fn is_indexable(path: &Path) -> bool {
    let extensions = [
        "txt", "md", "rs", "py", "js", "ts", "html", "css", "json",
        "toml", "yaml", "yml", "xml", "csv", "log", "sh", "bash",
        "c", "cpp", "h", "hpp", "java", "go", "rb", "php", "swift",
        "kt", "scala", "sql", "r", "m", "tex", "org", "rst",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| extensions.contains(&e))
        .unwrap_or(false)
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1_073_741_824 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    }
}

// ── Benchmark (unchanged except for ranking:: path) ───────────────────

fn run_benchmark(
    inv_index: &index::InvertedIndex,
    _doc_store: &store::DocumentStore,
    spell: &bktree::SpellCorrector,
) {
    let config = ranking::BM25Config::default();

    let bm25_queries = [
        "function", "data structure", "performance", "algorithm",
        "memory", "search", "compile", "type system", "async runtime",
        "consensus protocol", "hash function encryption", "gradient descent",
    ];

    println!("🏎️  BM25 Benchmark ({} queries):\n", bm25_queries.len());
    let mut total = std::time::Duration::ZERO;
    for q in &bm25_queries {
        let start = Instant::now();
        let results = ranking::search(inv_index, q, &config, 10);
        let elapsed = start.elapsed();
        total += elapsed;
        println!("  '{}' → {} results in {:.3?}", q, results.len(), elapsed);
    }
    println!(
        "  \x1b[32mTotal: {:.3?}, Avg: {:.3?}/query\x1b[0m\n",
        total,
        total / bm25_queries.len() as u32,
    );

    let spell_queries = [
        "concensus protocl", "memroy allocaton", "encyption algorthm", "databse indx",
    ];
    println!("📝 Spell Correction Benchmark ({} queries):\n", spell_queries.len());
    total = std::time::Duration::ZERO;
    for q in &spell_queries {
        let start = Instant::now();
        let correction = spell.correct_query(q);
        let elapsed = start.elapsed();
        total += elapsed;
        match correction {
            Some(c) => println!("  '{}' → '{}' in {:.3?}", q, c.corrected, elapsed),
            None => println!("  '{}' → (no correction) in {:.3?}", q, elapsed),
        }
    }
    println!(
        "  \x1b[32mTotal: {:.3?}, Avg: {:.3?}/query\x1b[0m",
        total,
        total / spell_queries.len() as u32,
    );
}
