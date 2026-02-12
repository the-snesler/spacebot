# MVP Roadmap

Tracking progress toward a working Spacebot that can hold a conversation, delegate work, manage memory, and connect to at least one messaging platform.

For each piece: reference IronClaw, OpenClaw, Nanobot, and Rig for inspiration, but make design decisions that align with Spacebot's architecture. Don't copy patterns that assume a monolithic session model.

---

## Current State

**What exists and works:**
- Project structure, all modules declared, module root pattern in place
- Error hierarchy (thiserror, top-level Error with domain #[from] wrappers)
- Config loading (env-based, compaction/channel defaults)
- Database connections (SQLite + LanceDB + redb, migration runner)
- LLM manager and SpacebotModel (direct HTTP to Anthropic/OpenAI)
- Memory types, SQLite store (full CRUD + associations), embedding (fastembed), hybrid search (FTS + graph + RRF), maintenance (decay/prune)
- StatusBlock (event-driven, renders to context string)
- SpacebotHook (tool events, leak detection regexes, tool nudging)
- Messaging trait with RPITIT + MessagingDyn companion
- Tool stubs: memory_save, memory_recall, shell, set_status
- main.rs with CLI, tracing, config, DB init, event loop, graceful shutdown

**What's stubbed or missing:**
- All 5 agent types (channel, branch, worker, compactor, cortex) are empty structs
- LanceDB vector storage (table management, embedding insert/query)
- Tools not wired to Rig's ToolServer — current ToolServerHandle is a placeholder
- No system prompts (prompts/ directory doesn't exist)
- No conversation history persistence
- No identity file loading
- Messaging adapters are empty
- tools/file.rs and tools/exec.rs declared but missing from disk
- No migrations written
- Secrets and settings stores are empty

---

## Phase 1: Fix the Foundation

Get the project compiling cleanly with no dead code or missing files.

- [ ] Create missing `src/tools/file.rs` and `src/tools/exec.rs` (at minimum stubs)
- [ ] Write initial SQLite migrations (memories table, associations table, conversations table, conversation_archives table)
- [ ] Verify `cargo check` passes with no errors
- [ ] Fix any import issues from the scaffold

**Reference:** IronClaw's migration structure for table schemas. Spacebot uses sqlx migrations, not a custom runner.

---

## Phase 2: LanceDB Vector Storage

The hybrid search pipeline needs real vector storage instead of the current stub.

- [ ] Implement `memory/lance.rs` — table creation, embedding insert, vector search (HNSW)
- [ ] Wire embedding generation into memory save flow (generate embedding on create, store in LanceDB)
- [ ] Connect vector results into `memory/search.rs` hybrid search (currently only uses FTS + graph)
- [ ] Test: save a memory, search for it by semantic similarity

**Reference:** IronClaw's pgvector HNSW config (`m=16, ef_construction=64`) for index parameters. Spacebot uses LanceDB instead of pgvector, but the search fusion (RRF with `k=60`) is the same. The search module already has RRF implemented — it just needs real vector results to fuse.

---

## Phase 3: Wire Tools to Rig

Replace the placeholder ToolServerHandle with Rig's actual ToolServer.

- [ ] Implement tools as Rig `Tool` trait impls (associated const NAME, Args/Output types, JsonSchema derives)
- [ ] Create shared ToolServer for channel/branch tools (reply, branch, spawn_worker, memory_save, route, cancel)
- [ ] Create per-worker ToolServer factory for task tools (shell, file, exec, set_status)
- [ ] Update AgentDeps to hold a real `rig::tool::server::ToolServerHandle`
- [ ] Implement `tools/file.rs` — read/write/list with workspace path guards
- [ ] Implement `tools/exec.rs` — subprocess execution with timeout

**Reference:** Rig's `Tool` trait uses `const NAME`, `type Args: Deserialize + JsonSchema`, `type Output: Serialize`. The `ToolServer::run()` consumes the server and returns a handle. IronClaw's workspace path guard pattern applies to file/exec tools — reject writes to identity/memory paths. Doc comments on tool input structs serve as LLM instructions.

---

## Phase 4: System Prompts and Identity

Create the prompt files and identity loading that give agents their behavior.

- [ ] Create `prompts/` directory
- [ ] Write `prompts/channel.md` — personality, delegation instructions, tool usage guide
- [ ] Write `prompts/branch.md` — thinking instructions, memory recall guidance
- [ ] Write `prompts/worker.md` — task execution instructions, status reporting
- [ ] Write `prompts/compactor.md` — summarization and memory extraction instructions
- [ ] Write `prompts/cortex.md` — system observation instructions
- [ ] Implement `identity/files.rs` — load SOUL.md, IDENTITY.md, USER.md from config dir
- [ ] Build context assembly in `conversation/context.rs` — combine prompt + identity + memories + status block

**Reference:** OpenClaw's skills-as-prompt-injections model for how to structure channel prompts. Nanobot's context building (~236 lines) as a simplicity target. IronClaw for the system prompt structure. Identity files are raw text injected into system prompts, not parsed.

---

## Phase 5: The Channel

The core user-facing agent. This is the MVP centerpiece.

- [ ] Implement Channel struct — owns history, agent, deps, status block, active branch/worker handles
- [ ] Build the message handling loop — receive InboundMessage, run agent.prompt() with history, emit reply
- [ ] Wire status block injection — prepend status to each prompt call
- [ ] Implement conversation history persistence (`conversation/history.rs`) — save/load from SQLite
- [ ] Fire-and-forget DB writes for message persistence (tokio::spawn, don't block the response)
- [ ] Handle streaming responses (agent.stream_prompt() with on_text_delta forwarding to messaging)
- [ ] Test: send a message to a channel, get a response back

**Reference:** IronClaw's agent loop for the overall structure, but without the Mutex-heavy locking — Spacebot channels own their state. Rig's `agent.prompt().with_history(&mut history).max_turns(5)` is the core call. IronClaw's fire-and-forget DB persistence pattern. The channel never blocks on branches, workers, or compaction.

---

## Phase 6: Branches

Context forking for thinking. The channel branches instead of doing heavy reasoning inline.

- [ ] Implement Branch struct — cloned history, independent agent, returns conclusion
- [ ] Implement branch spawning from channel — clone history, spawn tokio task, track JoinHandle
- [ ] Implement branch result injection — when branch completes, insert conclusion into channel history
- [ ] Implement branch concurrency limit (configurable max per channel)
- [ ] Handle stale branch results — branch forked 5 messages ago, conclusion may reference old context
- [ ] Wire branch tool for channel agent

**Reference:** No existing codebase has this exact model. IronClaw has workers but no context forking. The design is: `let branch_history = channel_history.clone()`, run independently, first-done-first-incorporated. Rig's history is `Vec<Message>` which is `Clone`.

---

## Phase 7: Workers

Independent task executors with no channel context.

- [ ] Implement Worker struct — fresh history, task-specific prompt, scoped tools, state machine
- [ ] Implement WorkerState transitions (Running → WaitingForInput/Done/Failed) with `can_transition_to()`
- [ ] Implement fire-and-forget workers — receive task, execute, return result
- [ ] Implement interactive workers — accept follow-up messages routed from channel
- [ ] Wire spawn_worker, route_to_worker, and cancel tools
- [ ] Worker status reporting via set_status tool → StatusBlock updates in channel

**Reference:** IronClaw's JobState machine for transition validation. Rig's `agent.prompt(&task).max_turns(50)` for workers. Interactive workers use repeated `.prompt()` calls with accumulated history. The worker abstraction should be pluggable — a Rig agent now, but the interface (receive task, report status, accept follow-ups, return result) should work for external processes later.

---

## Phase 8: The Compactor

Programmatic context monitor. Not an LLM process itself — it watches a number and spawns workers.

- [ ] Implement Compactor — monitors channel context token count
- [ ] Implement tiered thresholds (>80% background, >85% aggressive, >95% emergency truncation)
- [ ] Background compaction worker — summarize old turns + extract memories in one pass
- [ ] Emergency truncation — drop oldest turns without LLM, keep N recent
- [ ] Pre-compaction archiving — write raw transcript to conversation_archives before summarizing
- [ ] Non-blocking swap — replace old turns with summary while channel continues

**Reference:** IronClaw's tiered compaction (80/85/95 thresholds). OpenClaw's memory flush before compaction. NanoClaw's pre-compact transcript archiving. The novel challenge is the non-blocking swap — no existing codebase does compaction without blocking the conversation.

---

## Phase 9: Webhook Messaging Adapter

Get a real messaging path working. Webhook is simplest — no external API dependencies.

- [ ] Implement WebhookAdapter — HTTP server that accepts POST messages and returns responses
- [ ] Implement MessagingManager.start_all() — spawn adapters, merge inbound streams via select_all
- [ ] Implement outbound routing — responses flow from channel → manager → correct adapter
- [ ] Wire the full path: HTTP POST → InboundMessage → Channel → agent response → OutboundResponse → HTTP response
- [ ] Test: curl a message in, get a response back

**Reference:** IronClaw's Channel trait and ChannelManager with `futures::stream::select_all()` for fan-in. The StatusUpdate enum vocabulary (Thinking, ToolStarted, etc.) is already defined in lib.rs. NanoClaw's message bus pattern for decoupling.

---

## Phase 10: End-to-End Integration

Wire everything together into a running system.

- [ ] main.rs orchestration — init config, DB, LLM, memory, tools, messaging, event loop
- [ ] Event routing — ProcessEvent fan-in from all agents, dispatch to appropriate handlers
- [ ] Channel lifecycle — create on first message, persist across restarts, resume from DB
- [ ] Test the full loop: message in → channel → branch for thinking → worker for task → memory save → response out
- [ ] Graceful shutdown — broadcast signal, drain in-flight work, close DB connections

---

## Post-MVP

Not blocking the first working version, but next in line.

- **Cortex** — system-level observer, memory consolidation, decay management. No codebase has a reference for this; design from first principles.
- **Heartbeats** — scheduled tasks with fresh channels. IronClaw's circuit breaker (3 consecutive failures → disable) applies here.
- **Telegram adapter** — real messaging platform integration.
- **Discord adapter** — thread-based conversations map naturally to channels.
- **Secrets store** — AES-256-GCM with per-secret HKDF derivation. Copy IronClaw's scheme exactly.
- **Settings store** — redb key-value with env > DB > default resolution.
- **Memory graph traversal during recall** — walk typed edges (Updates, Contradicts, CausedBy) during search. No codebase does this; novel design.
- **Multi-channel identity coherence** — same soul across conversations, cortex consolidates memories across channels.
