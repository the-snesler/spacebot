const API_BASE = "/api";

export interface StatusResponse {
	status: string;
	pid: number;
	uptime_seconds: number;
}

export interface ChannelInfo {
	agent_id: string;
	id: string;
	platform: string;
	display_name: string | null;
	is_active: boolean;
	last_activity_at: string;
	created_at: string;
}

export interface ChannelsResponse {
	channels: ChannelInfo[];
}

export type ProcessType = "channel" | "branch" | "worker";

export interface InboundMessageEvent {
	type: "inbound_message";
	agent_id: string;
	channel_id: string;
	sender_id: string;
	text: string;
}

export interface OutboundMessageEvent {
	type: "outbound_message";
	agent_id: string;
	channel_id: string;
	text: string;
}

export interface TypingStateEvent {
	type: "typing_state";
	agent_id: string;
	channel_id: string;
	is_typing: boolean;
}

export interface WorkerStartedEvent {
	type: "worker_started";
	agent_id: string;
	channel_id: string | null;
	worker_id: string;
	task: string;
}

export interface WorkerStatusEvent {
	type: "worker_status";
	agent_id: string;
	channel_id: string | null;
	worker_id: string;
	status: string;
}

export interface WorkerCompletedEvent {
	type: "worker_completed";
	agent_id: string;
	channel_id: string | null;
	worker_id: string;
	result: string;
}

export interface BranchStartedEvent {
	type: "branch_started";
	agent_id: string;
	channel_id: string;
	branch_id: string;
	description: string;
}

export interface BranchCompletedEvent {
	type: "branch_completed";
	agent_id: string;
	channel_id: string;
	branch_id: string;
	conclusion: string;
}

export interface ToolStartedEvent {
	type: "tool_started";
	agent_id: string;
	channel_id: string | null;
	process_type: ProcessType;
	process_id: string;
	tool_name: string;
}

export interface ToolCompletedEvent {
	type: "tool_completed";
	agent_id: string;
	channel_id: string | null;
	process_type: ProcessType;
	process_id: string;
	tool_name: string;
}

export type ApiEvent =
	| InboundMessageEvent
	| OutboundMessageEvent
	| TypingStateEvent
	| WorkerStartedEvent
	| WorkerStatusEvent
	| WorkerCompletedEvent
	| BranchStartedEvent
	| BranchCompletedEvent
	| ToolStartedEvent
	| ToolCompletedEvent;

async function fetchJson<T>(path: string): Promise<T> {
	const response = await fetch(`${API_BASE}${path}`);
	if (!response.ok) {
		throw new Error(`API error: ${response.status}`);
	}
	return response.json();
}

export interface TimelineMessage {
	type: "message";
	id: string;
	role: "user" | "assistant";
	sender_name: string | null;
	sender_id: string | null;
	content: string;
	created_at: string;
}

export interface TimelineBranchRun {
	type: "branch_run";
	id: string;
	description: string;
	conclusion: string | null;
	started_at: string;
	completed_at: string | null;
}

export interface TimelineWorkerRun {
	type: "worker_run";
	id: string;
	task: string;
	result: string | null;
	status: string;
	started_at: string;
	completed_at: string | null;
}

export type TimelineItem = TimelineMessage | TimelineBranchRun | TimelineWorkerRun;

export interface MessagesResponse {
	items: TimelineItem[];
}

export interface WorkerStatusInfo {
	id: string;
	task: string;
	status: string;
	started_at: string;
	notify_on_complete: boolean;
	tool_calls: number;
}

export interface BranchStatusInfo {
	id: string;
	started_at: string;
	description: string;
}

export interface CompletedItemInfo {
	id: string;
	item_type: "Branch" | "Worker";
	description: string;
	completed_at: string;
	result_summary: string;
}

export interface StatusBlockSnapshot {
	active_workers: WorkerStatusInfo[];
	active_branches: BranchStatusInfo[];
	completed_items: CompletedItemInfo[];
}

/** channel_id -> StatusBlockSnapshot */
export type ChannelStatusResponse = Record<string, StatusBlockSnapshot>;

export interface AgentInfo {
	id: string;
	workspace: string;
	context_window: number;
	max_turns: number;
	max_concurrent_branches: number;
}

export interface AgentsResponse {
	agents: AgentInfo[];
}

export const api = {
	status: () => fetchJson<StatusResponse>("/status"),
	agents: () => fetchJson<AgentsResponse>("/agents"),
	channels: () => fetchJson<ChannelsResponse>("/channels"),
	channelMessages: (channelId: string, limit = 20) =>
		fetchJson<MessagesResponse>(
			`/channels/messages?channel_id=${encodeURIComponent(channelId)}&limit=${limit}`,
		),
	channelStatus: () => fetchJson<ChannelStatusResponse>("/channels/status"),
	eventsUrl: `${API_BASE}/events`,
};
