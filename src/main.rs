//! Spacebot CLI entry point.

use anyhow::Context as _;
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "spacebot")]
#[command(about = "A Rust agentic system with dedicated processes for every task")]
struct Cli {
    /// Path to config file (optional)
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,
    
    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();
    
    // Initialize logging
    let filter = if cli.debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };
    
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
    
    tracing::info!("Starting Spacebot...");
    
    // Load configuration
    let config = if let Some(config_path) = cli.config {
        spacebot::config::Config::load_from_path(&config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))?
    } else {
        spacebot::config::Config::load()
            .with_context(|| "failed to load configuration from environment")?
    };
    
    tracing::info!(data_dir = %config.data_dir.display(), "Configuration loaded");
    
    // Initialize databases
    let db = spacebot::db::Db::connect(&config.data_dir)
        .await
        .with_context(|| "failed to connect to databases")?;
    
    tracing::info!("Database connections established");
    
    // Initialize LLM manager
    let llm_manager = Arc::new(
        spacebot::llm::LlmManager::new(config.llm.clone())
            .await
            .with_context(|| "failed to initialize LLM manager")?
    );
    
    tracing::info!("LLM manager initialized");
    
    // Initialize memory store
    let memory_store = spacebot::memory::MemoryStore::new(db.sqlite.clone());
    
    tracing::info!("Memory store initialized");
    
    // Create shared dependencies
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(64);
    let tool_server = spacebot::tools::ToolServerHandle::new();
    
    let _deps = spacebot::AgentDeps {
        memory_store,
        llm_manager,
        tool_server,
        event_tx: event_tx.clone(),
    };
    
    // Start event processing loop
    let event_loop = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            tracing::debug!(?event, "Process event received");
            // Event handling will be implemented in Phase 3
        }
    });
    
    tracing::info!("Spacebot started successfully");
    
    // Wait for event loop to complete (or Ctrl-C)
    tokio::select! {
        _ = event_loop => {
            tracing::info!("Event loop ended");
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutdown signal received");
        }
    }
    
    // Graceful shutdown
    tracing::info!("Shutting down...");
    db.close().await;
    
    tracing::info!("Spacebot stopped");
    Ok(())
}

use std::sync::Arc;
