//! Heartbeat scheduler for timer-based tasks.

use crate::error::Result;
use crate::{ProcessEvent, ChannelId, AgentDeps};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration, Interval};
use chrono::Timelike;

/// A heartbeat is a scheduled task that fires at intervals.
#[derive(Debug, Clone)]
pub struct Heartbeat {
    pub id: String,
    pub prompt: String,
    pub interval_secs: u64,
    pub delivery_target: String,
    pub active_hours: Option<(u8, u8)>, // (start_hour, end_hour) in 24h format
    pub enabled: bool,
    pub consecutive_failures: u32,
    pub last_run: Option<chrono::DateTime<chrono::Utc>>,
}

/// Heartbeat configuration for storage.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HeartbeatConfig {
    pub id: String,
    pub prompt: String,
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    pub delivery_target: String,
    pub active_hours: Option<(u8, u8)>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_interval() -> u64 {
    3600 // 1 hour default
}

fn default_true() -> bool {
    true
}

/// Scheduler that manages heartbeat timers.
pub struct Scheduler {
    heartbeats: Arc<RwLock<HashMap<String, Heartbeat>>>,
    timers: Arc<RwLock<HashMap<String, tokio::task::JoinHandle<()>>>>,
    deps: AgentDeps,
    event_tx: mpsc::Sender<HeartbeatEvent>,
}

/// Events from the scheduler.
#[derive(Debug, Clone)]
pub enum HeartbeatEvent {
    /// A heartbeat fired and needs to be executed.
    Fired { heartbeat_id: String },
    /// A heartbeat was disabled due to failures.
    CircuitBroken { heartbeat_id: String },
}

impl Scheduler {
    /// Create a new heartbeat scheduler.
    pub fn new(deps: AgentDeps) -> (Self, mpsc::Receiver<HeartbeatEvent>) {
        let (event_tx, event_rx) = mpsc::channel(64);
        
        let scheduler = Self {
            heartbeats: Arc::new(RwLock::new(HashMap::new())),
            timers: Arc::new(RwLock::new(HashMap::new())),
            deps,
            event_tx,
        };
        
        (scheduler, event_rx)
    }
    
    /// Register a new heartbeat.
    pub async fn register(&self, config: HeartbeatConfig) -> Result<()> {
        let heartbeat = Heartbeat {
            id: config.id.clone(),
            prompt: config.prompt,
            interval_secs: config.interval_secs,
            delivery_target: config.delivery_target,
            active_hours: config.active_hours,
            enabled: config.enabled,
            consecutive_failures: 0,
            last_run: None,
        };
        
        {
            let mut heartbeats = self.heartbeats.write().await;
            heartbeats.insert(config.id.clone(), heartbeat);
        }
        
        // Start the timer if enabled
        if config.enabled {
            self.start_timer(&config.id, config.interval_secs).await?;
        }
        
        tracing::info!(heartbeat_id = %config.id, "heartbeat registered");
        
        Ok(())
    }
    
    /// Start a timer for a heartbeat.
    async fn start_timer(&self, heartbeat_id: &str, interval_secs: u64) -> Result<()> {
        let heartbeat_id_owned = heartbeat_id.to_string();
        let event_tx = self.event_tx.clone();
        let heartbeats = self.heartbeats.clone();
        
        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs));
            
            loop {
                ticker.tick().await;
                
                // Check if still enabled
                let should_fire = {
                    let heartbeats = heartbeats.read().await;
                    if let Some(hb) = heartbeats.get(&heartbeat_id_owned) {
                        if !hb.enabled {
                            break;
                        }
                        
                        // Check active hours
                        if let Some((start, end)) = hb.active_hours {
                            let current_hour = chrono::Local::now().hour() as u8;
                            if current_hour < start || current_hour > end {
                                continue; // Skip this tick
                            }
                        }
                        
                        true
                    } else {
                        break; // Heartbeat removed
                    }
                };
                
                if should_fire {
                    let _ = event_tx.send(HeartbeatEvent::Fired {
                        heartbeat_id: heartbeat_id_owned.clone(),
                    }).await;
                }
            }
        });
        
        {
            let mut timers = self.timers.write().await;
            timers.insert(heartbeat_id.to_string(), handle);
        }
        
        Ok(())
    }
    
    /// Get a heartbeat by ID.
    pub async fn get(&self, id: &str) -> Option<Heartbeat> {
        let heartbeats = self.heartbeats.read().await;
        heartbeats.get(id).cloned()
    }
    
    /// Disable a heartbeat (circuit breaker after failures).
    pub async fn disable(&self, id: &str) -> Result<()> {
        let mut heartbeats = self.heartbeats.write().await;
        if let Some(hb) = heartbeats.get_mut(id) {
            hb.enabled = false;
            tracing::warn!(heartbeat_id = %id, "heartbeat disabled (circuit broken)");
        }
        Ok(())
    }
    
    /// Record a failure for circuit breaker logic.
    pub async fn record_failure(&self, id: &str) -> Result<()> {
        const MAX_FAILURES: u32 = 3;
        
        let mut heartbeats = self.heartbeats.write().await;
        if let Some(hb) = heartbeats.get_mut(id) {
            hb.consecutive_failures += 1;
            
            if hb.consecutive_failures >= MAX_FAILURES {
                hb.enabled = false;
                drop(heartbeats);
                let _ = self.event_tx.send(HeartbeatEvent::CircuitBroken {
                    heartbeat_id: id.to_string(),
                }).await;
            }
        }
        
        Ok(())
    }
    
    /// Shutdown all timers.
    pub async fn shutdown(&self) {
        let mut timers = self.timers.write().await;
        for (id, handle) in timers.drain() {
            handle.abort();
            tracing::debug!(heartbeat_id = %id, "heartbeat timer stopped");
        }
    }
}

/// Run a heartbeat when it fires.
pub async fn run_heartbeat(
    heartbeat: &Heartbeat,
    deps: AgentDeps,
) -> Result<()> {
    tracing::info!(heartbeat_id = %heartbeat.id, "running heartbeat");
    
    // In real implementation:
    // 1. Create a fresh short-lived channel
    // 2. Give it the heartbeat prompt
    // 3. Run it like a normal channel (with branching, workers, etc.)
    // 4. Deliver result to the target if there's anything to report
    
    // For now, just log
    tracing::info!(
        heartbeat_id = %heartbeat.id,
        prompt = %heartbeat.prompt,
        "heartbeat executed"
    );
    
    Ok(())
}
