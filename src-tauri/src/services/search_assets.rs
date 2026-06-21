//! RRF fusion engine — FTS5 + LanceDB symmetric retrieval
use std::collections::HashMap;

const RRF_K: f64 = 60.0;

pub fn rrf_fusion(fts5: &[(String, f64)], vector: &[(String, f64)], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for (rank, (id, _)) in fts5.iter().enumerate() { *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + (rank + 1) as f64); }
    for (rank, (id, _)) in vector.iter().enumerate() { *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + (rank + 1) as f64); }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}
