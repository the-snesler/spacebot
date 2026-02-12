//! Messaging trait and dynamic dispatch companion.

use crate::error::Result;
use crate::{InboundMessage, OutboundResponse, StatusUpdate};
use std::pin::Pin;
use futures::Stream;

/// Message stream type.
pub type InboundStream = Pin<Box<dyn Stream<Item = InboundMessage> + Send>>;

/// Static trait for messaging adapters.
/// Use this for type-safe implementations.
pub trait Messaging: Send + Sync + 'static {
    /// Unique name for this adapter.
    fn name(&self) -> &str;
    
    /// Start the adapter and return inbound message stream.
    fn start(&self) -> impl std::future::Future<Output = Result<InboundStream>> + Send;
    
    /// Send a response to a message.
    fn respond(
        &self,
        message: &InboundMessage,
        response: OutboundResponse,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
    
    /// Send a status update.
    fn send_status(
        &self,
        message: &InboundMessage,
        status: StatusUpdate,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
    
    /// Broadcast a message.
    fn broadcast(
        &self,
        target: &str,
        response: OutboundResponse,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
    
    /// Health check.
    fn health_check(&self) -> impl std::future::Future<Output = Result<()>> + Send;
    
    /// Graceful shutdown.
    fn shutdown(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
}

/// Dynamic trait for runtime polymorphism.
/// Use this when you need `Arc<dyn MessagingDyn>` for storing different adapters.
pub trait MessagingDyn: Send + Sync + 'static {
    fn name(&self) -> &str;
    
    fn start<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = Result<InboundStream>> + Send + 'a>>;
    
    fn respond<'a>(
        &'a self,
        message: &'a InboundMessage,
        response: OutboundResponse,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;
    
    fn send_status<'a>(
        &'a self,
        message: &'a InboundMessage,
        status: StatusUpdate,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;
    
    fn broadcast<'a>(
        &'a self,
        target: &'a str,
        response: OutboundResponse,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;
    
    fn health_check<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;
    
    fn shutdown<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;
}

/// Blanket implementation: any type implementing Messaging automatically implements MessagingDyn.
impl<T: Messaging> MessagingDyn for T {
    fn name(&self) -> &str {
        Messaging::name(self)
    }
    
    fn start<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = Result<InboundStream>> + Send + 'a>> {
        Box::pin(Messaging::start(self))
    }
    
    fn respond<'a>(
        &'a self,
        message: &'a InboundMessage,
        response: OutboundResponse,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(Messaging::respond(self, message, response))
    }
    
    fn send_status<'a>(
        &'a self,
        message: &'a InboundMessage,
        status: StatusUpdate,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(Messaging::send_status(self, message, status))
    }
    
    fn broadcast<'a>(
        &'a self,
        target: &'a str,
        response: OutboundResponse,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(Messaging::broadcast(self, target, response))
    }
    
    fn health_check<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(Messaging::health_check(self))
    }
    
    fn shutdown<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(Messaging::shutdown(self))
    }
}
