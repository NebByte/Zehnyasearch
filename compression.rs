/// Index compression using Variable Byte (VByte) Encoding.
///
/// The idea: postings lists are sorted sequences of doc IDs like [3, 7, 15, 42, 100].
/// Instead of storing each as a full u32 (4 bytes), we:
///   1. Delta-encode: store the gaps [3, 4, 8, 27, 58]
///   2. VByte-encode each gap: small numbers use 1 byte, large numbers use more
///
/// VByte encoding: use 7 bits per byte for data, 1 bit as continuation flag.
///   - High bit = 1 means "more bytes follow"
///   - High bit = 0 means "this is the last byte"
///
/// This typically compresses postings lists by 50-75%.

/// Encode a single u32 into VByte format, appending bytes to `out`.
pub fn vbyte_encode_u32(mut value: u32, out: &mut Vec<u8>) {
    // Collect 7-bit groups, least significant first
    if value == 0 {
        out.push(0); // Just a zero byte with high bit clear
        return;
    }

    let mut temp = Vec::new();
    while value > 0 {
        temp.push((value & 0x7F) as u8); // low 7 bits
        value >>= 7;
    }

    // Write all but the last with continuation bit set
    for i in 0..temp.len() - 1 {
        out.push(temp[i] | 0x80); // set high bit = "more coming"
    }
    // Last byte: high bit clear = "done"
    out.push(temp[temp.len() - 1]); // high bit already clear
}

/// Decode a single VByte-encoded u32 from `data` starting at `pos`.
/// Returns (decoded_value, new_pos).
pub fn vbyte_decode_u32(data: &[u8], pos: usize) -> (u32, usize) {
    let mut result: u32 = 0;
    let mut shift = 0;
    let mut i = pos;

    loop {
        if i >= data.len() {
            break;
        }
        let byte = data[i];
        result |= ((byte & 0x7F) as u32) << shift;
        i += 1;
        if byte & 0x80 == 0 {
            break; // last byte
        }
        shift += 7;
    }

    (result, i)
}

/// Delta-encode a sorted list of doc IDs, then VByte-compress.
pub fn compress_doc_ids(doc_ids: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();

    // First: encode the count
    vbyte_encode_u32(doc_ids.len() as u32, &mut out);

    // Then: delta-encode and compress
    let mut prev = 0u32;
    for &id in doc_ids {
        let delta = id - prev;
        vbyte_encode_u32(delta, &mut out);
        prev = id;
    }

    out
}

/// Decompress a VByte-encoded, delta-encoded list of doc IDs.
pub fn decompress_doc_ids(data: &[u8]) -> Vec<u32> {
    if data.is_empty() {
        return vec![];
    }

    let mut pos = 0;

    // Read count
    let (count, new_pos) = vbyte_decode_u32(data, pos);
    pos = new_pos;

    // Read deltas and reconstruct
    let mut doc_ids = Vec::with_capacity(count as usize);
    let mut prev = 0u32;

    for _ in 0..count {
        let (delta, new_pos) = vbyte_decode_u32(data, pos);
        pos = new_pos;
        let id = prev + delta;
        doc_ids.push(id);
        prev = id;
    }

    doc_ids
}

/// Compress a list of positions (also sorted, also delta-encodable).
pub fn compress_positions(positions: &[u32]) -> Vec<u8> {
    compress_doc_ids(positions) // Same algorithm works for any sorted u32 list
}

pub fn decompress_positions(data: &[u8]) -> Vec<u32> {
    decompress_doc_ids(data)
}

/// Compressed posting: stores tf + compressed positions.
#[derive(Clone, Debug)]
pub struct CompressedPosting {
    pub doc_id: u32,
    pub term_freq: u32,
    pub positions_compressed: Vec<u8>,
}

impl CompressedPosting {
    pub fn from_positions(doc_id: u32, positions: &[u32]) -> Self {
        Self {
            doc_id,
            term_freq: positions.len() as u32,
            positions_compressed: compress_positions(positions),
        }
    }

    pub fn decompress_positions(&self) -> Vec<u32> {
        decompress_positions(&self.positions_compressed)
    }
}

/// Compute compression statistics for a set of postings.
pub struct CompressionStats {
    pub original_bytes: usize,
    pub compressed_bytes: usize,
    pub ratio: f64,
    pub savings_percent: f64,
}

pub fn compute_stats(
    postings_map: &std::collections::HashMap<String, Vec<crate::index::Posting>>,
) -> CompressionStats {
    let mut original_bytes = 0usize;
    let mut compressed_bytes = 0usize;

    for postings in postings_map.values() {
        // Original: each posting = doc_id(4) + term_freq(4) + positions(4 each)
        for p in postings {
            original_bytes += 4 + 4 + p.positions.len() * 4;
        }

        // Compressed: delta-encoded doc IDs + compressed positions per posting
        let doc_ids: Vec<u32> = postings.iter().map(|p| p.doc_id).collect();
        compressed_bytes += compress_doc_ids(&doc_ids).len();

        for p in postings {
            compressed_bytes += compress_positions(&p.positions).len();
            compressed_bytes += 4; // term_freq stored as-is
        }
    }

    let ratio = if original_bytes > 0 {
        compressed_bytes as f64 / original_bytes as f64
    } else {
        1.0
    };

    CompressionStats {
        original_bytes,
        compressed_bytes,
        ratio,
        savings_percent: (1.0 - ratio) * 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vbyte_roundtrip() {
        let values = [0, 1, 127, 128, 255, 1000, 65535, 1_000_000, u32::MAX];
        for &v in &values {
            let mut encoded = Vec::new();
            vbyte_encode_u32(v, &mut encoded);
            let (decoded, _) = vbyte_decode_u32(&encoded, 0);
            assert_eq!(v, decoded, "Failed roundtrip for {}", v);
        }
    }

    #[test]
    fn test_small_values_are_compact() {
        let mut buf = Vec::new();
        vbyte_encode_u32(50, &mut buf);
        assert_eq!(buf.len(), 1, "Values < 128 should be 1 byte");

        buf.clear();
        vbyte_encode_u32(200, &mut buf);
        assert_eq!(buf.len(), 2, "Values 128-16383 should be 2 bytes");
    }

    #[test]
    fn test_doc_id_compression_roundtrip() {
        let ids = vec![3, 7, 15, 42, 100, 500, 501, 502, 10000];
        let compressed = compress_doc_ids(&ids);
        let decompressed = decompress_doc_ids(&compressed);
        assert_eq!(ids, decompressed);

        // Should be much smaller than 9 * 4 = 36 bytes
        println!(
            "Original: {} bytes, Compressed: {} bytes",
            ids.len() * 4,
            compressed.len()
        );
        assert!(compressed.len() < ids.len() * 4);
    }
}
