# Heartbeats

Scheduled recurring tasks. A heartbeat is a prompt that fires on a timer, gets a fresh channel to work in, and delivers the result to a messaging target.

## Why Not Just One Timer

OpenClaw has a single heartbeat: one HEARTBEAT.md file, one timer, one LLM call that tries to do everything in a single turn. It runs in the main session, competes for context, and if it does too much work it triggers compaction. Two things can't run at different intervals. Everything is serialized.

Spacebot has multiple independent heartbeats. Each one is a database row with its own interval, its own prompt, and its own delivery target. When a heartbeat fires, it gets a fresh short-lived channel — the same kind of channel that handles user conversations, with full branching and worker capabilities. Multiple heartbeats run concurrently without blocking each other.

## How It Works

```
Heartbeat "check-email" fires (every 30m, active 09:00-17:00)
    → Scheduler creates a fresh Channel
    → Channel receives the heartbeat prompt as a synthetic message
    → Channel runs the LLM loop (can branch, spawn workers, use tools)
    → Channel produces OutboundResponse::Text
    → Scheduler collects the response
    → Scheduler delivers it via MessagingManager::broadcast("discord", "123456789")
    → Channel shuts down

Heartbeat "daily-summary" fires (every 24h)
    → Same flow, different prompt, different target
    → Runs independently even if "check-email" is still in-flight
```

If the channel produces no text output, nothing is delivered. No magic tokens, no "HEARTBEAT_OK" — if there's nothing to say, the heartbeat is silent.

## Storage

Two SQLite tables in the agent's database.

### heartbeats

The configuration table. One row per heartbeat.

```sql
CREATE TABLE heartbeats (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    interval_secs INTEGER NOT NULL DEFAULT 3600,
    delivery_target TEXT NOT NULL,
    active_start_hour INTEGER,
    active_end_hour INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

| Column | Description |
|--------|-------------|
| `id` | Short unique name (e.g. "check-email", "daily-summary") |
| `prompt` | The instruction to execute on each run |
| `interval_secs` | Seconds between runs (3600 = hourly, 86400 = daily) |
| `delivery_target` | Where to send results, format `adapter:target` (e.g. `discord:123456789`) |
| `active_start_hour` | Optional start of active window (0-23, 24h local time) |
| `active_end_hour` | Optional end of active window (0-23, 24h local time) |
| `enabled` | Flipped to 0 by the circuit breaker after consecutive failures |

### heartbeat_executions

Audit log. One row per execution attempt.

```sql
CREATE TABLE heartbeat_executions (
    id TEXT PRIMARY KEY,
    heartbeat_id TEXT NOT NULL,
    executed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success INTEGER NOT NULL,
    result_summary TEXT,
    FOREIGN KEY (heartbeat_id) REFERENCES heartbeats(id) ON DELETE CASCADE
);
```

## Delivery Targets

The `delivery_target` field uses the format `adapter:target`:

| Format | Meaning |
|--------|---------|
| `discord:123456789` | Send to Discord channel ID 123456789 |
| `discord:987654321` | Send to a different Discord channel |
| `webhook:some-endpoint` | Send via webhook adapter |

The adapter name maps to a registered messaging adapter. The target string is adapter-specific — for Discord, it's a channel ID parsed to u64. Delivery goes through `MessagingManager::broadcast()`, which is the proactive (non-reply) message path.

## Creation Paths

Heartbeats enter the system three ways.

### 1. Config File

Defined in `config.toml` under an agent. Seeded into the database on startup (upsert — won't overwrite runtime changes to existing IDs).

```toml
[[agents]]
id = "main"
default = true

[[agents.heartbeats]]
id = "daily-summary"
prompt = "Summarize what happened across all conversations today."
interval_secs = 86400
delivery_target = "discord:123456789012345678"
active_start_hour = 9
active_end_hour = 10

[[agents.heartbeats]]
id = "check-inbox"
prompt = "Check the inbox for anything that needs attention."
interval_secs = 1800
delivery_target = "discord:123456789012345678"
active_start_hour = 9
active_end_hour = 17
```

### 2. Conversational (via the heartbeat tool)

A user says "check my email every day at 9am" and the channel LLM calls the `heartbeat` tool:

```json
{
  "action": "create",
  "id": "check-email",
  "prompt": "Check the user's email inbox and summarize any important messages.",
  "interval_secs": 86400,
  "delivery_target": "discord:123456789",
  "active_start_hour": 9,
  "active_end_hour": 10
}
```

The tool persists to the database and registers with the running scheduler immediately — no restart needed.

The tool also supports `list` (show all active heartbeats) and `delete` (remove by ID).

### 3. Programmatic

Any code with access to `HeartbeatStore` and `Scheduler` can create heartbeats. The cortex could create them based on observed patterns. A future CLI command could manage them directly.

## Active Hours

The active window uses 24-hour local time. If `active_start_hour` and `active_end_hour` are both set, the heartbeat only fires within that window.

Midnight wrapping is handled: a window of `22:00-06:00` means "10pm to 6am" — the heartbeat fires if the current hour is >= 22 or < 6.

If active hours are not set, the heartbeat runs at all hours.

Active hours don't affect the timer interval — the timer still ticks at `interval_secs`. When a tick lands outside the active window, it's skipped. The next tick happens at the normal interval, not "as soon as the window opens."

## Circuit Breaker

If a heartbeat fails 3 consecutive times, it's automatically disabled:

1. `enabled` is set to `false` in the in-memory scheduler state
2. The change is persisted to SQLite via `HeartbeatStore::update_enabled()`
3. The timer loop exits
4. A warning is logged

A "failure" is any error from `run_heartbeat()` — LLM errors, channel failures, delivery failures. A successful execution (even one that produces no output) resets the failure counter to 0.

Disabled heartbeats are not loaded on restart (the store query filters `WHERE enabled = 1`). To re-enable a disabled heartbeat, update the database row directly or re-seed it from config with `enabled = true`.

## Execution Flow

When the scheduler fires a heartbeat:

1. **Create channel** — A fresh `Channel` is constructed with the agent's deps, prompts, identity, and skills. It gets a unique ID of `heartbeat:{heartbeat_id}`.

2. **Send prompt** — A synthetic `InboundMessage` with source `"heartbeat"` is sent to the channel. The message contains the heartbeat's prompt text.

3. **Run** — The channel processes the message through its normal LLM loop. It can use all channel tools (reply, branch, spawn_worker, memory_save, etc).

4. **Collect** — The scheduler reads from the channel's `response_tx`. Text responses are collected. Status updates and stream events are ignored.

5. **Timeout** — If the channel doesn't finish within 120 seconds, it's aborted.

6. **Log** — The execution is recorded in `heartbeat_executions` with success status and a summary of the output.

7. **Deliver** — If there's non-empty text, it's sent to the delivery target via `MessagingManager::broadcast()`. If the output is empty, delivery is skipped.

8. **Teardown** — The channel's sender is dropped after sending the prompt, so the channel's event loop exits naturally after processing the single message.

## Scheduler Lifecycle

The scheduler is created per-agent after messaging adapters are initialized (it needs `MessagingManager` for delivery). On startup:

1. A `HeartbeatStore` is created from the agent's SQLite pool
2. Config-defined heartbeats are seeded into the database (upsert)
3. All enabled heartbeats are loaded from the database
4. A `Scheduler` is created with a `HeartbeatContext` (agent deps + messaging)
5. Each heartbeat is registered, starting its timer loop
6. The `heartbeat` tool is registered on the agent's `ToolServerHandle`

Timer loops skip the first tick — heartbeats wait one full interval before their first execution. This prevents a burst of activity on startup.

On shutdown, all timer handles are aborted.

## Module Layout

```
src/
├── heartbeat.rs            → heartbeat/
│   ├── scheduler.rs        — Scheduler, Heartbeat, HeartbeatConfig, HeartbeatContext,
│   │                         DeliveryTarget, run_heartbeat(), timer loops
│   └── store.rs            — HeartbeatStore: save, load_all, delete, update_enabled,
│                             log_execution (SQLite)
│
├── tools/
│   └── heartbeat.rs        — HeartbeatTool: create/list/delete (Rig tool)
│
└── main.rs                 — scheduler creation, config seeding, tool registration,
                              shutdown
```

## Key Types

```rust
/// Runtime state for a registered heartbeat.
pub struct Heartbeat {
    pub id: String,
    pub prompt: String,
    pub interval_secs: u64,
    pub delivery_target: DeliveryTarget,
    pub active_hours: Option<(u8, u8)>,
    pub enabled: bool,
    pub consecutive_failures: u32,
}

/// Parsed from "adapter:target" format.
pub struct DeliveryTarget {
    pub adapter: String,   // e.g. "discord"
    pub target: String,    // e.g. "123456789"
}

/// Serializable config for storage and TOML parsing.
pub struct HeartbeatConfig {
    pub id: String,
    pub prompt: String,
    pub interval_secs: u64,
    pub delivery_target: String,  // raw "adapter:target" string
    pub active_hours: Option<(u8, u8)>,
    pub enabled: bool,
}

/// Everything needed to execute a heartbeat.
pub struct HeartbeatContext {
    pub deps: AgentDeps,
    pub system_prompt: String,
    pub identity_context: String,
    pub branch_system_prompt: String,
    pub worker_system_prompt: String,
    pub compactor_prompt: String,
    pub browser_config: BrowserConfig,
    pub screenshot_dir: PathBuf,
    pub skills: Arc<SkillSet>,
    pub messaging_manager: Arc<MessagingManager>,
    pub store: Arc<HeartbeatStore>,
}
```

## What's Not Implemented Yet

- **Cron expressions** — only fixed intervals for now. A heartbeat that should run "at 9am daily" currently uses `interval_secs: 86400` with `active_start_hour: 9, active_end_hour: 10`. Real cron scheduling would be more precise.
- **Error backoff** — on failure, the next attempt happens at the normal interval. Progressive backoff (30s → 1m → 5m → 15m → 60m) would reduce cost during outages.
- **Cross-run context** — each heartbeat starts with a blank history. A heartbeat that needs to know what it found last time would need to use memory recall.
- **Cortex management** — the cortex should be able to observe heartbeat health, re-enable circuit-broken heartbeats, and create new heartbeats based on patterns.
- **CLI management** — `spacebot heartbeats list`, `spacebot heartbeats create`, etc.
