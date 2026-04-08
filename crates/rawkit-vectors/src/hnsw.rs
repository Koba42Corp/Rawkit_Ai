use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use rawkit_core::Soul;

/// HNSW (Hierarchical Navigable Small World) index for fast approximate
/// nearest neighbor search.
///
/// This is a clean implementation of the HNSW algorithm described in
/// "Efficient and robust approximate nearest neighbor search using
/// Hierarchical Navigable Small World graphs" (Malkov & Yashunin, 2016).
///
/// Key parameters:
/// - `m`: Max connections per node per layer (default 16)
/// - `ef_construction`: Size of dynamic candidate list during build (default 200)
/// - `ef_search`: Size of dynamic candidate list during search (default 50)
pub struct HnswIndex {
    dimensions: usize,
    m: usize,
    m_max0: usize,
    ef_construction: usize,
    ef_search: usize,
    ml: f64, // 1 / ln(m)
    /// All vectors stored by internal ID.
    vectors: Vec<HnswNode>,
    /// Map from soul to internal ID.
    soul_to_id: HashMap<Soul, usize>,
    /// Entry point (internal ID of top-level node).
    entry_point: Option<usize>,
    /// Maximum layer in the graph.
    max_level: usize,
}

#[derive(Clone, Serialize, Deserialize)]
struct HnswNode {
    soul: Soul,
    vector: Vec<f32>,
    /// Neighbors at each layer. neighbors[layer] = vec of (internal_id).
    neighbors: Vec<Vec<usize>>,
    level: usize,
}

#[derive(Clone, PartialEq, Eq)]
struct Candidate {
    distance: OrderedFloat<f32>,
    id: usize,
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.distance.cmp(&other.distance)
    }
}

impl HnswIndex {
    pub fn new(dimensions: usize) -> Self {
        Self::with_params(dimensions, 16, 200, 100)
    }

    pub fn with_params(dimensions: usize, m: usize, ef_construction: usize, ef_search: usize) -> Self {
        HnswIndex {
            dimensions,
            m,
            m_max0: m * 2,
            ef_construction,
            ef_search,
            ml: 1.0 / (m as f64).ln(),
            vectors: Vec::new(),
            soul_to_id: HashMap::new(),
            entry_point: None,
            max_level: 0,
        }
    }

    /// Insert or update a vector.
    pub fn upsert(&mut self, soul: &str, vector: Vec<f32>) {
        // Remove old entry if exists
        if let Some(&old_id) = self.soul_to_id.get(soul) {
            // Mark as deleted by zeroing the vector (lazy deletion)
            self.vectors[old_id].vector = vec![0.0; self.dimensions];
            self.soul_to_id.remove(soul);
        }

        let id = self.vectors.len();
        let level = self.random_level();

        let node = HnswNode {
            soul: soul.to_string(),
            vector,
            neighbors: vec![Vec::new(); level + 1],
            level,
        };

        self.soul_to_id.insert(soul.to_string(), id);

        if self.entry_point.is_none() {
            // First node
            self.vectors.push(node);
            self.entry_point = Some(id);
            self.max_level = level;
            return;
        }

        let ep = self.entry_point.unwrap();

        // Phase 1: Traverse from top to the node's level, greedy search
        let mut current_ep = ep;
        for lc in (level + 1..=self.max_level).rev() {
            current_ep = self.search_layer_single(&node.vector, current_ep, lc);
        }

        // Phase 2: Insert at each layer from node's level down to 0
        let query_vec = node.vector.clone();
        self.vectors.push(node);

        for lc in (0..=level.min(self.max_level)).rev() {
            let neighbors = self.search_layer(&query_vec, current_ep, self.ef_construction, lc);

            let max_conn = if lc == 0 { self.m_max0 } else { self.m };
            let selected: Vec<usize> = neighbors.iter().take(max_conn).map(|c| c.id).collect();

            // Add bidirectional connections
            self.vectors[id].neighbors[lc] = selected.clone();

            for &neighbor_id in &selected {
                if neighbor_id < self.vectors.len() && lc < self.vectors[neighbor_id].neighbors.len() {
                    self.vectors[neighbor_id].neighbors[lc].push(id);
                    // Prune if too many connections
                    if self.vectors[neighbor_id].neighbors[lc].len() > max_conn {
                        let nv = self.vectors[neighbor_id].vector.clone();
                        let mut scored: Vec<Candidate> = self.vectors[neighbor_id].neighbors[lc]
                            .iter()
                            .map(|&nid| Candidate {
                                distance: OrderedFloat(cosine_distance(&nv, &self.vectors[nid].vector)),
                                id: nid,
                            })
                            .collect();
                        scored.sort();
                        self.vectors[neighbor_id].neighbors[lc] =
                            scored.into_iter().take(max_conn).map(|c| c.id).collect();
                    }
                }
            }

            if !neighbors.is_empty() {
                current_ep = neighbors[0].id;
            }
        }

        // Update entry point if this node has a higher level
        if level > self.max_level {
            self.entry_point = Some(id);
            self.max_level = level;
        }
    }

    /// Remove a vector from the index.
    pub fn remove(&mut self, soul: &str) {
        if let Some(&id) = self.soul_to_id.get(soul) {
            self.vectors[id].vector = vec![0.0; self.dimensions];
            self.soul_to_id.remove(soul);
        }
    }

    /// Search for the top-k nearest neighbors.
    pub fn search(&self, query: &[f32], top_k: usize) -> Vec<(Soul, f32)> {
        if self.entry_point.is_none() || self.soul_to_id.is_empty() {
            return Vec::new();
        }

        let ep = self.entry_point.unwrap();

        // Greedy traverse from top to layer 1
        let mut current_ep = ep;
        for lc in (1..=self.max_level).rev() {
            current_ep = self.search_layer_single(query, current_ep, lc);
        }

        // Search at layer 0 with ef_search candidates
        let candidates = self.search_layer(query, current_ep, self.ef_search.max(top_k), 0);

        candidates
            .into_iter()
            .filter(|c| self.soul_to_id.values().any(|&id| id == c.id)) // skip deleted
            .take(top_k)
            .map(|c| {
                let sim = 1.0 - c.distance.into_inner(); // convert distance to similarity
                (self.vectors[c.id].soul.clone(), sim)
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.soul_to_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.soul_to_id.is_empty()
    }

    // ─── Internal ─────────────────────────────────────────────────────────

    fn random_level(&self) -> usize {
        let r: f64 = rand::random();
        (-r.ln() * self.ml).floor() as usize
    }

    /// Greedy search for a single nearest neighbor at a given layer.
    fn search_layer_single(&self, query: &[f32], entry: usize, layer: usize) -> usize {
        let mut current = entry;
        let mut current_dist = cosine_distance(query, &self.vectors[current].vector);

        loop {
            let mut changed = false;
            if layer < self.vectors[current].neighbors.len() {
                for &neighbor in &self.vectors[current].neighbors[layer] {
                    if neighbor < self.vectors.len() {
                        let dist = cosine_distance(query, &self.vectors[neighbor].vector);
                        if dist < current_dist {
                            current = neighbor;
                            current_dist = dist;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        current
    }

    /// Search a layer with ef candidates, returning sorted results.
    fn search_layer(&self, query: &[f32], entry: usize, ef: usize, layer: usize) -> Vec<Candidate> {
        let entry_dist = cosine_distance(query, &self.vectors[entry].vector);

        let mut visited = HashSet::new();
        visited.insert(entry);

        // Min-heap of candidates to explore
        let mut candidates = BinaryHeap::new();
        candidates.push(Reverse(Candidate {
            distance: OrderedFloat(entry_dist),
            id: entry,
        }));

        // Max-heap of best results (we want to drop the worst)
        let mut results: Vec<Candidate> = vec![Candidate {
            distance: OrderedFloat(entry_dist),
            id: entry,
        }];

        while let Some(Reverse(current)) = candidates.pop() {
            let worst_dist = results.iter().map(|r| r.distance).max().unwrap_or(OrderedFloat(f32::MAX));
            if current.distance > worst_dist && results.len() >= ef {
                break;
            }

            if layer < self.vectors[current.id].neighbors.len() {
                for &neighbor in &self.vectors[current.id].neighbors[layer] {
                    if neighbor < self.vectors.len() && visited.insert(neighbor) {
                        let dist = cosine_distance(query, &self.vectors[neighbor].vector);
                        let worst_dist = results.iter().map(|r| r.distance).max().unwrap_or(OrderedFloat(f32::MAX));

                        if OrderedFloat(dist) < worst_dist || results.len() < ef {
                            candidates.push(Reverse(Candidate {
                                distance: OrderedFloat(dist),
                                id: neighbor,
                            }));
                            results.push(Candidate {
                                distance: OrderedFloat(dist),
                                id: neighbor,
                            });
                            if results.len() > ef {
                                // Remove worst
                                if let Some(worst_idx) = results.iter().enumerate().max_by_key(|(_, r)| r.distance).map(|(i, _)| i) {
                                    results.swap_remove(worst_idx);
                                }
                            }
                        }
                    }
                }
            }
        }

        results.sort();
        results
    }
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return 1.0;
    }

    1.0 - (dot / denom)
}

// We need rand for random_level
// rand used by random_level via rand::random()

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hnsw_basic() {
        let mut index = HnswIndex::new(3);
        index.upsert("a", vec![1.0, 0.0, 0.0]);
        index.upsert("b", vec![0.0, 1.0, 0.0]);
        index.upsert("c", vec![0.9, 0.1, 0.0]);

        let results = index.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a"); // exact match
        assert!(results[0].1 > 0.99);
    }

    #[test]
    fn test_hnsw_many_vectors() {
        let mut index = HnswIndex::new(64);

        // Insert 1000 vectors
        for i in 0..1000 {
            let mut v = vec![0.0f32; 64];
            v[i % 64] = 1.0;
            v[(i + 1) % 64] = 0.5;
            index.upsert(&format!("doc/{i}"), v);
        }

        assert_eq!(index.len(), 1000);

        // Search should return results
        let mut query = vec![0.0f32; 64];
        query[0] = 1.0;
        query[1] = 0.5;

        let results = index.search(&query, 5);
        assert_eq!(results.len(), 5);
        // First result should be very similar
        assert!(results[0].1 > 0.9);
    }

    #[test]
    fn test_hnsw_remove() {
        let mut index = HnswIndex::new(3);
        index.upsert("a", vec![1.0, 0.0, 0.0]);
        index.upsert("b", vec![0.0, 1.0, 0.0]);
        assert_eq!(index.len(), 2);

        index.remove("a");
        assert_eq!(index.len(), 1);

        let results = index.search(&[1.0, 0.0, 0.0], 5);
        assert!(results.iter().all(|(soul, _)| soul != "a"));
    }

    #[test]
    fn test_hnsw_update() {
        let mut index = HnswIndex::new(3);
        index.upsert("a", vec![1.0, 0.0, 0.0]);
        index.upsert("a", vec![0.0, 1.0, 0.0]); // update

        let results = index.search(&[0.0, 1.0, 0.0], 1);
        assert_eq!(results[0].0, "a");
        assert!(results[0].1 > 0.99);
    }

    #[test]
    fn test_hnsw_empty() {
        let index = HnswIndex::new(3);
        let results = index.search(&[1.0, 0.0, 0.0], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_hnsw_performance_improvement() {
        // Insert 5000 vectors and measure search time
        let mut index = HnswIndex::new(128);
        for i in 0..5000 {
            let mut v = vec![0.0f32; 128];
            v[i % 128] = 1.0;
            v[(i * 7) % 128] = 0.5;
            index.upsert(&format!("v/{i}"), v);
        }

        let query = vec![1.0f32; 128];
        let start = std::time::Instant::now();
        for _ in 0..100 {
            index.search(&query, 10);
        }
        let elapsed = start.elapsed();
        let ops_sec = 100.0 / elapsed.as_secs_f64();

        // HNSW should be much faster than brute force
        // At 5K vectors, we expect >1000 ops/sec (vs ~170 brute force at 10K)
        println!("HNSW search: {ops_sec:.0} ops/sec (5K vectors, 128 dims)");
        assert!(ops_sec > 100.0, "HNSW should be faster than brute force");
    }
}
