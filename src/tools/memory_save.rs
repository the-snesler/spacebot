//! Memory save tool for channels and branches.

use crate::error::Result;
use crate::memory::{Memory, MemoryStore, MemoryType};
use crate::memory::types::{CreateMemoryInput, CreateAssociationInput, RelationType};
use std::sync::Arc;

/// Save a memory to the store.
pub async fn memory_save(
    memory_store: &MemoryStore,
    input: CreateMemoryInput,
) -> Result<String> {
    // Create the memory
    let mut memory = Memory::new(&input.content, input.memory_type);
    
    if let Some(importance) = input.importance {
        memory = memory.with_importance(importance);
    }
    
    if let Some(source) = input.source {
        memory = memory.with_source(source);
    }
    
    if let Some(channel_id) = input.channel_id {
        memory = memory.with_channel_id(channel_id);
    }
    
    // Save to database
    memory_store.save(&memory).await?;
    
    // Create associations
    for assoc_input in input.associations {
        let association = crate::memory::types::Association::new(
            &memory.id,
            &assoc_input.target_id,
            assoc_input.relation_type,
        ).with_weight(assoc_input.weight);
        
        memory_store.create_association(&association).await?;
    }
    
    // Store embedding if provided
    if let Some(_embedding) = input.embedding {
        // TODO: Store in LanceDB when table is ready
    }
    
    Ok(memory.id)
}

/// Convenience function for simple fact saving.
pub async fn save_fact(
    memory_store: &MemoryStore,
    content: impl Into<String>,
    channel_id: Option<crate::ChannelId>,
) -> Result<String> {
    let input = CreateMemoryInput {
        content: content.into(),
        memory_type: MemoryType::Fact,
        importance: None,
        source: None,
        channel_id,
        embedding: None,
        associations: vec![],
    };
    
    memory_save(memory_store, input).await
}
