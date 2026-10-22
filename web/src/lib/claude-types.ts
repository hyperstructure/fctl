// The parts of Claude Code's stream-json messages the UI reads. Messages are
// stored verbatim, so these types are deliberately open; narrow with the
// guards below.

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

export interface TextBlock {
  type: "text";
  text: string;
}
export interface ThinkingBlock {
  type: "thinking";
  thinking: string;
}
export interface ToolUseBlock {
  type: "tool_use";
  id: string;
  name: string;
  input: Record<string, Json>;
}
export interface ToolResultBlock {
  type: "tool_result";
  tool_use_id: string;
  content?: string | Array<{ type: string; text?: string }>;
  is_error?: boolean;
}
export type ContentBlock = TextBlock | ThinkingBlock | ToolUseBlock | ToolResultBlock | { type: string };

export interface Usage {
  input_tokens?: number;
  output_tokens?: number;
  cache_read_input_tokens?: number;
  cache_creation_input_tokens?: number;
}

/** Any stream-json message. */
export interface ClaudeMessage {
  type: string;
  subtype?: string;
  uuid?: string;
  session_id?: string;
  [k: string]: unknown;
}

export interface InitMessage extends ClaudeMessage {
  type: "system";
  subtype: "init";
  model: string;
  cwd: string;
  tools: string[];
  apiKeySource: string;
  claude_code_version?: string;
  permissionMode?: string;
}

export interface AssistantMessage extends ClaudeMessage {
  type: "assistant";
  message: {
    id: string;
    model?: string;
    content: ContentBlock[];
    usage?: Usage;
    stop_reason?: string | null;
  };
  parent_tool_use_id: string | null;
  error?: string;
}

export interface UserToolMessage extends ClaudeMessage {
  type: "user";
  message: { role: "user"; content: string | ContentBlock[] };
  parent_tool_use_id: string | null;
  tool_use_result?: unknown;
}

export interface ResultMessage extends ClaudeMessage {
  type: "result";
  subtype: string;
  duration_ms: number;
  num_turns: number;
  is_error: boolean;
  result?: string;
  usage?: Usage;
}

/**
 * Claude Code's word on its background work and subagents:
 * `task_started`, `task_progress`, `task_notification` (as it ends, for
 * any reason).
 */
export interface TaskMessage extends ClaudeMessage {
  type: "system";
  subtype: string;
  task_id?: string;
  /** The tool call that started it. */
  tool_use_id?: string;
  /** `local_agent` for a subagent, `local_bash` for a shell command, … */
  task_type?: string;
  is_backgrounded?: boolean;
  /** How it ended: `completed`, `failed`, `stopped`. */
  status?: string;
  /** A subagent's last words, as it ended. */
  summary?: string;
  usage?: { duration_ms?: number; tool_uses?: number; total_tokens?: number };
}

export interface RateLimitMessage extends ClaudeMessage {
  type: "rate_limit_event";
  rate_limit_info: {
    status: string;
    resetsAt?: number;
    rateLimitType?: string;
    utilization?: number;
    isUsingOverage?: boolean;
  };
}

export type RawStreamEvent =
  | { type: "message_start"; message: { id: string; model?: string } }
  | {
      type: "content_block_start";
      index: number;
      content_block: { type: string; name?: string; id?: string };
    }
  | {
      type: "content_block_delta";
      index: number;
      delta: { type: string; text?: string; thinking?: string; partial_json?: string };
    }
  | { type: "content_block_stop"; index: number }
  | { type: "message_delta"; delta: { stop_reason?: string } }
  | { type: "message_stop" };

export interface ClaudeStreamEvent {
  type: "stream_event" | "tool_progress";
  event?: RawStreamEvent;
  parent_tool_use_id: string | null;
  tool_use_id?: string;
  tool_name?: string;
  elapsed_time_seconds?: number;
  uuid?: string;
}

export const isInit = (m: ClaudeMessage): m is InitMessage => m.type === "system" && m.subtype === "init";
export const isAssistant = (m: ClaudeMessage): m is AssistantMessage => m.type === "assistant";
export const isUserTool = (m: ClaudeMessage): m is UserToolMessage => m.type === "user";
export const isResult = (m: ClaudeMessage): m is ResultMessage => m.type === "result";
export const isRateLimit = (m: ClaudeMessage): m is RateLimitMessage => m.type === "rate_limit_event";
export const isTask = (m: ClaudeMessage): m is TaskMessage =>
  m.type === "system" && typeof m.subtype === "string" && m.subtype.startsWith("task_");

export function resultText(content: ToolResultBlock["content"]): string {
  if (typeof content === "string") return content;
  if (!content) return "";
  return content
    .map((c) => c.text ?? "")
    .filter(Boolean)
    .join("\n");
}
