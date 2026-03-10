//! Memory Agent Example
//!
//! This example demonstrates the usage of the claw-kernel memory system:
//! - NgramEmbedder: Creates 64-dimensional text embeddings
//! - SqliteMemoryStore: Persistent SQLite-based memory storage
//! - MemoryWorker: Background worker for async memory archiving
//! - MemoryItem: Creating and storing memory entries
//! - Semantic Search: Finding similar memories using embeddings

use claw_kernel::prelude::*;
// Explicit imports for memory types (also in prelude since v1.4.1)
use claw_kernel::memory::sqlite::SqliteMemoryStore;
use claw_kernel::provider::embedding::NgramEmbedder;
use claw_kernel::memory::worker::MemoryWorker;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Memory Agent Example ===\n");

    // 1. Create NgramEmbedder (64-dimensional embeddings)
    let embedder = NgramEmbedder::new();
    println!("✓ Created NgramEmbedder (dimensions: {})", embedder.dimensions());

    // 2. Create SqliteMemoryStore with temporary file database
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("memory_agent_example.db");
    let store = Arc::new(SqliteMemoryStore::open(&db_path)?);
    println!("✓ Created SqliteMemoryStore at: {:?}", db_path);

    // 3. Create EventBus and start MemoryWorker
    let event_bus = Arc::new(EventBus::new());
    let (worker, worker_handle) = MemoryWorker::new(Arc::clone(&store), Arc::clone(&event_bus));
    let _worker_task = worker.start();
    println!("✓ MemoryWorker started\n");

    // 4. Store several memory items with embeddings
    println!("--- Storing Memories ---");
    
    let memories = vec![
        ("用户喜欢 Rust 编程语言", vec!["preference", "programming"]),
        ("用户是一名软件开发者", vec!["profession", "identity"]),
        ("用户喜欢使用命令行工具", vec!["preference", "tools"]),
        ("用户正在学习 AI 技术", vec!["learning", "ai"]),
        ("用户喜欢咖啡", vec!["preference", "lifestyle"]),
    ];

    let agent_id = AgentId::new("memory-agent");
    let mut memory_items = Vec::new();

    for (content, tags) in memories {
        // Create embedding for the content
        let embedding = embedder.embed(content);
        
        // Create MemoryItem with embedding and tags
        let item = MemoryItem::new("memory-agent", content)
            .with_embedding(embedding)
            .with_tags(tags.iter().map(|t| t.to_string()).collect());
        
        println!("  Storing: \"{}\" [tags: {}]", content, tags.join(", "));
        memory_items.push(item);
    }

    // Archive memories through the worker
    worker_handle.archive(agent_id.clone(), memory_items).await?;
    
    // Wait for worker to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("✓ Memories archived\n");

    // 5. Perform semantic search
    println!("--- Semantic Search ---");
    
    let queries = vec![
        "编程语言",
        "开发工作",
        "饮料偏好",
        "学习新技术",
    ];

    for query in queries {
        // Embed the query
        let query_embedding = embedder.embed(query);
        
        // Search for similar memories
        let results = store.semantic_search(&query_embedding, 3).await?;
        
        println!("\n  Query: \"{}\"", query);
        if results.is_empty() {
            println!("    (no results found)");
        } else {
            for (i, item) in results.iter().enumerate() {
                // Calculate similarity score
                let item_emb = item.embedding.as_ref().unwrap();
                let similarity = cosine_similarity(&query_embedding, item_emb);
                println!(
                    "    {}. \"{}\" (similarity: {:.4})",
                    i + 1,
                    item.content,
                    similarity
                );
            }
        }
    }

    // 6. Demonstrate namespace usage
    println!("\n--- Namespace Statistics ---");
    let usage = store.namespace_usage("memory-agent").await?;
    println!("  Namespace 'memory-agent' usage: {} bytes", usage);

    // 7. Cleanup
    println!("\n--- Cleanup ---");
    drop(worker_handle);
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Delete temporary database file
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
        println!("✓ Deleted temporary database: {:?}", db_path);
    }

    println!("\n=== Example Complete ===");
    Ok(())
}

/// Calculate cosine similarity between two vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}
