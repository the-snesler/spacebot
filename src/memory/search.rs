//! Hybrid search: vector + FTS + RRF + graph traversal.

use crate::error::Result;
use crate::memory::types::{Memory, MemorySearchResult, RelationType};
use crate::memory::MemoryStore;
use std::collections::HashMap;
use std::sync::Arc;

/// Hybrid search configuration.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Maximum number of results from each source (vector, fts, graph).
    pub max_results_per_source: usize,
    /// RRF k parameter (typically 60).
    pub rrf_k: f64,
    /// Minimum score threshold for results.
    pub min_score: f32,
    /// Maximum graph traversal depth.
    pub max_graph_depth: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_results_per_source: 50,
            rrf_k: 60.0,
            min_score: 0.3,
            max_graph_depth: 2,
        }
    }
}

/// Perform hybrid search across all memory sources.
pub async fn hybrid_search(
    memory_store: &MemoryStore,
    query: &str,
    config: &SearchConfig,
) -> Result<Vec<MemorySearchResult>> {
    // Collect results from different sources
    let mut vector_results = Vec::new();
    let mut fts_results = Vec::new();
    let mut graph_results = Vec::new();
    
    // 1. Full-text search (SQLite-based)
    let fts_matches = memory_store.search_content(query, config.max_results_per_source as i64).await?;
    for (memory, score) in fts_matches {
        fts_results.push(ScoredMemory { memory, score: score as f64 });
    }
    
    // 2. Graph traversal from high-importance memories
    // Get identity and high-importance memories as starting points
    let seed_memories = memory_store.get_high_importance(0.8, 20).await?;
    
    for seed in seed_memories {
        // Check if seed is semantically related to query via simple keyword matching
        if query.to_lowercase().split_whitespace().any(|term| {
            seed.content.to_lowercase().contains(term)
        }) {
            graph_results.push(ScoredMemory { 
                memory: seed.clone(), 
                score: seed.importance as f64
            });
            
            // Traverse graph to find related memories
            traverse_graph(memory_store, &seed.id, config.max_graph_depth, &mut graph_results).await?;
        }
    }
    
    // 3. Merge results using Reciprocal Rank Fusion (RRF)
    let fused_results = reciprocal_rank_fusion(
        &vector_results,
        &fts_results,
        &graph_results,
        config.rrf_k,
    );
    
    // Convert to MemorySearchResult with ranks
    let results: Vec<MemorySearchResult> = fused_results
        .into_iter()
        .enumerate()
        .map(|(rank, scored)| MemorySearchResult {
            memory: scored.memory,
            score: scored.score as f32,
            rank: rank + 1,
        })
        .filter(|r| r.score >= config.min_score)
        .take(config.max_results_per_source)
        .collect();
    
    Ok(results)
}

/// Simple scored memory for internal use.
#[derive(Debug, Clone)]
struct ScoredMemory {
    memory: Memory,
    score: f64,
}

/// Traverse the memory graph to find related memories (iterative to avoid async recursion).
async fn traverse_graph(
    memory_store: &MemoryStore,
    start_id: &str,
    max_depth: usize,
    results: &mut Vec<ScoredMemory>,
) -> Result<()> {
    use std::collections::VecDeque;
    
    // Queue of (memory_id, current_depth)
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    
    queue.push_back((start_id.to_string(), 0));
    visited.insert(start_id.to_string());
    
    while let Some((current_id, depth)) = queue.pop_front() {
        if depth > max_depth {
            continue;
        }
        
        let associations = memory_store.get_associations(&current_id).await?;
        
        for assoc in associations {
            // Get the related memory
            let related_id = if assoc.source_id == current_id {
                &assoc.target_id
            } else {
                &assoc.source_id
            };
            
            if visited.contains(related_id) {
                continue;
            }
            visited.insert(related_id.clone());
            
            if let Some(memory) = memory_store.load(related_id).await? {
                // Score based on relation type and weight
                let type_multiplier = match assoc.relation_type {
                    RelationType::Updates => 1.5,
                    RelationType::CausedBy | RelationType::ResultOf => 1.3,
                    RelationType::RelatedTo => 1.0,
                    RelationType::Contradicts => 0.5,
                    RelationType::PartOf => 0.8,
                };
                
                let score = memory.importance as f64 * assoc.weight as f64 * type_multiplier;
                
                results.push(ScoredMemory { memory: memory.clone(), score });
                
                // Add to queue for RelatedTo and PartOf relations
                if matches!(assoc.relation_type, RelationType::RelatedTo | RelationType::PartOf) {
                    queue.push_back((related_id.clone(), depth + 1));
                }
            }
        }
    }
    
    Ok(())
}

/// Reciprocal Rank Fusion to combine results from multiple sources.
/// RRF score = sum(1 / (k + rank)) for each list where the item appears.
fn reciprocal_rank_fusion(
    vector_results: &[ScoredMemory],
    fts_results: &[ScoredMemory],
    graph_results: &[ScoredMemory],
    k: f64,
) -> Vec<ScoredMemory> {
    // Build a map of memory ID to RRF score
    let mut rrf_scores: HashMap<String, (f64, Memory)> = HashMap::new();
    
    // Add vector results
    for (rank, scored) in vector_results.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank as f64 + 1.0));
        let entry = rrf_scores.entry(scored.memory.id.clone())
            .or_insert((0.0, scored.memory.clone()));
        entry.0 += rrf_score;
    }
    
    // Add FTS results
    for (rank, scored) in fts_results.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank as f64 + 1.0));
        let entry = rrf_scores.entry(scored.memory.id.clone())
            .or_insert((0.0, scored.memory.clone()));
        entry.0 += rrf_score;
    }
    
    // Add graph results
    for (rank, scored) in graph_results.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank as f64 + 1.0));
        let entry = rrf_scores.entry(scored.memory.id.clone())
            .or_insert((0.0, scored.memory.clone()));
        entry.0 += rrf_score;
    }
    
    // Convert to vec and sort by RRF score
    let mut fused: Vec<ScoredMemory> = rrf_scores
        .into_iter()
        .map(|(_, (score, memory))| ScoredMemory { memory, score })
        .collect();
    
    fused.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    
    fused
}

/// Curate search results to return only the most relevant.
pub fn curate_results(results: &[MemorySearchResult], max_results: usize) -> Vec<&Memory> {
    results
        .iter()
        .take(max_results)
        .map(|r| &r.memory)
        .collect()
}
