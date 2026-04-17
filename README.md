# Zehnyasearch

**A full-featured search engine built from scratch in Rust.**

Crawl the web or index local files, then search with BM25 ranking, PageRank, boolean queries, fuzzy matching, and spell correction — all implemented by hand, no search-engine libraries.

```bash
# Crawl, save, and serve a web UI — three steps
cargo run --release -- https://example.com --save corpus.bin
cargo run --release -- --load corpus.bin --serve
# → http://localhost:8080
```

## Why build it from scratch?

Lucene, Tantivy, Meilisearch, and Elasticsearch are excellent — but they hide the interesting parts. Zehnyasearch is the opposite: every component is implemented by hand so you can read the code and see exactly how search works end-to-end. The inverted index, the BM25 formula, the PageRank power iteration, the Porter stemmer, the BK-tree for spell correction, the VByte postings compression, the robots.txt-respecting crawler — all of it lives in ~2,500 lines of readable Rust.

No `lucene`, no `tantivy`, no `meilisearch` crate. Just:

- `ureq` for HTTP requests
- `scraper` for HTML parsing
- `url` for URL handling
- `bincode` + `serde` for corpus serialization
- `tiny_http` for the web UI

Everything else — the search engine itself — is in this repo.

## Features

- **BM25 Ranking** — the Okapi BM25 formula with configurable k1/b parameters and smoothed IDF
- **PageRank** — iterative power-method computation over the crawl link graph, blended into result scoring
- **Phrase Search** — exact consecutive-term matching with positional index lookups
- **Boolean Queries** — recursive-descent parser supporting `AND`, `OR`, `NOT`, and parenthesized grouping
- **Fuzzy Search** — Damerau-Levenshtein distance with edit tolerance up to 2
- **Spell Correction** — BK-Tree (metric tree) for sub-linear approximate string lookup, with automatic "Did you mean?" suggestions
- **Variable-Byte Compression** — delta-encoded, VByte-compressed postings lists (50-75% size reduction)
- **Web Crawler** — polite crawler with `robots.txt` support, configurable depth/delay/domain scope
- **Local File Indexing** — index directories of source code, docs, and text files (35+ file types)
- **HTTP Search UI** — clean, Google-style web interface served by a built-in HTTP server
- **CLI REPL** — interactive terminal search with colored output and inline stats
- **Corpus Persistence** — save/load the full index to binary for instant restarts
- **Porter Stemmer** — hand-rolled implementation for English morphological normalization

## Architecture

```
main.rs          CLI entry point, argument parsing, REPL, benchmarks
crawler.rs       Web crawler with robots.txt, rate limiting, link extraction
server.rs        HTTP server and search UI (HTML/CSS inline)
index.rs         Inverted index with positional postings
store.rs         Document storage and snippet generation
corpus.rs        Binary serialization/deserialization of the full index
ranking.rs       BM25 scoring, phrase search, title/URL/PageRank reranking
pagerank.rs      PageRank via power iteration
tokenizer.rs     Tokenization, stop-word removal, Porter stemmer
boolean.rs       Boolean query parser (recursive descent) and evaluator
fuzzy.rs         Fuzzy matching with Damerau-Levenshtein distance
bktree.rs        BK-Tree for fast spell correction
compression.rs   Variable-byte encoding for postings compression
```

## Getting Started

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2021+)

### Build

```bash
git clone https://github.com/NebByte/Zehnyasearch.git
cd Zehnyasearch
cargo build --release
```

The release binary lands at `./target/release/search-engine`.

### Usage

**Crawl a website and search it:**
```bash
./target/release/search-engine https://example.com --save corpus.bin
```

**Index a local directory:**
```bash
./target/release/search-engine ./my-documents
```

**Load a saved corpus and start the web UI:**
```bash
./target/release/search-engine --load corpus.bin --serve --port 8080
```
Then open [http://localhost:8080](http://localhost:8080) in your browser.

**Incremental crawl (add pages to an existing corpus):**
```bash
./target/release/search-engine --load corpus.bin https://another-site.com --save corpus.bin
```

**Run the built-in benchmark:**
```bash
./target/release/search-engine --load corpus.bin --bench
```

### Crawl Options

| Flag | Default | Description |
|------|---------|-------------|
| `--max-pages N` | 50 | Maximum pages to crawl |
| `--depth N` | 2 | Maximum link-follow depth |
| `--delay MS` | 500 | Milliseconds between requests |
| `--cross-domain` | off | Allow following links to other domains |
| `--save PATH` | — | Save corpus to disk after indexing |
| `--serve` | off | Start the HTTP search UI |
| `--port N` | 8080 | HTTP server port |
| `--bench` | off | Run performance benchmarks |

## Search Modes

In both the REPL and the web UI:

| Mode | Syntax | Example |
|------|--------|---------|
| Standard (BM25) | `query` | `hash table` |
| Phrase | `phrase:query` | `phrase:binary search tree` |
| Boolean | `bool:expression` | `bool:TCP AND socket NOT UDP` |
| Fuzzy | `fuzzy:query` | `fuzzy:concensus` |
| Spell check | `spell:query` | `spell:algorthm` |

Boolean queries support `AND`, `OR`, `NOT`, and parentheses:
```
bool:memory AND (heap OR stack) NOT garbage
bool:(leader AND election) OR consensus
```

## How It Works

1. **Crawling/Indexing** — The crawler fetches pages respecting `robots.txt`, extracts text and links using CSS selectors, and feeds documents into the indexer. Local file indexing walks directories and reads supported file types.

2. **Tokenization** — Text is lowercased, split on non-alphanumeric characters, filtered through a stop-word list, and stemmed using a hand-written Porter stemmer.

3. **Inverted Index** — Each term maps to a postings list containing document IDs, term frequencies, and exact positions (for phrase search). Postings are delta-encoded and compressed with VByte encoding.

4. **Ranking** — Queries are scored with BM25, then reranked with title-match boosts (1.6x), URL-match boosts (1.2x), and a logarithmic PageRank blend.

5. **Spell Correction** — A BK-Tree indexes the vocabulary using Damerau-Levenshtein distance. On zero results, the engine automatically corrects the query and re-searches.

## Supported File Types

When indexing local directories, the following extensions are recognized:

`txt` `md` `rs` `py` `js` `ts` `html` `css` `json` `toml` `yaml` `yml` `xml` `csv` `log` `sh` `bash` `c` `cpp` `h` `hpp` `java` `go` `rb` `php` `swift` `kt` `scala` `sql` `r` `m` `tex` `org` `rst`

## Project Layout

```
Zehnyasearch/
├── Cargo.toml
├── main.rs           CLI entry point and REPL
├── server.rs         HTTP server and search UI
├── crawler.rs        Web crawler
├── index.rs          Inverted index
├── store.rs          Document store
├── corpus.rs         Binary serialization
├── tokenizer.rs      Porter stemmer and tokenization
├── ranking.rs        BM25 scoring
├── pagerank.rs       PageRank
├── boolean.rs        Boolean query parser
├── fuzzy.rs          Damerau-Levenshtein
├── bktree.rs         Spell-correction BK-tree
└── compression.rs    VByte postings encoding
```

## Contributing

Issues and pull requests are welcome. Good starter ideas:

- Additional tokenizers (non-English languages)
- More compression schemes for postings (Simple9, PForDelta)
- Ranking improvements (query-dependent features, proximity scoring)
- Incremental indexing and deletion support
- A proper benchmark corpus and latency/recall measurements

## License

[MIT](LICENSE) — free to use, modify, and learn from.
