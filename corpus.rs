/// Serialize/deserialize the full search corpus so we can save an expensive
/// crawl and reload it for later queries (or an HTTP server).

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use serde::{Serialize, Deserialize};

use std::collections::HashMap;

use crate::index::InvertedIndex;
use crate::store::DocumentStore;

#[derive(Serialize, Deserialize)]
pub struct Corpus {
    pub index: InvertedIndex,
    pub store: DocumentStore,
    pub link_graph: Vec<Vec<u32>>,
    pub pageranks: Vec<f32>,
    /// URL → doc_id mapping so incremental crawls skip already-indexed pages
    /// and can resolve cross-links to old documents.
    pub url_map: HashMap<String, u32>,
}

impl Corpus {
    pub fn save(&self, path: &Path) -> std::io::Result<u64> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        bincode::serialize_into(&mut writer, self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::metadata(path).map(|m| m.len())
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        bincode::deserialize_from(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}
