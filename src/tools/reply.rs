//! Reply tool for sending messages to users (channel only).

use crate::{InboundMessage, OutboundResponse};

/// Send a reply to a user message.
pub async fn reply(
    message: &InboundMessage,
    content: impl Into<String>,
) -> anyhow::Result<()> {
    let _response = OutboundResponse::Text(content.into());
    
    // In real implementation, this would route through MessagingManager
    // For now, just log it
    tracing::info!(
        conversation_id = %message.conversation_id,
        "sending reply to user"
    );
    
    Ok(())
}

/// Send a streaming reply chunk.
pub async fn reply_stream_chunk(
    message: &InboundMessage,
    chunk: impl Into<String>,
) -> anyhow::Result<()> {
    let _chunk = chunk.into();
    
    tracing::debug!(
        conversation_id = %message.conversation_id,
        "sending stream chunk"
    );
    
    Ok(())
}
