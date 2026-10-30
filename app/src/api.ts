import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useRef } from "react";

// Mirrors the Rust types in app/src-tauri/src/{host,commands}.rs and
// deskhand-policy. Keep in sync when those change.

export type Tier = "read" | "reversible" | "irreversible";

export type ConfirmReason =
  | { kind: "irreversible" }
  | { kind: "user_override" }
  | { kind: "tainted_context"; sources: string[] };

export type Provider = "anthropic" | "openai_compatible";

export interface Status {
  provider: Provider;
  model: string;
  /** Set whenever a non-default endpoint is configured; API keys go there. */
  base_url: string | null;
  has_anthropic_key: boolean;
  has_openai_key: boolean;
  needs_key: boolean;
  busy: boolean;
  hotkey: string;
  hotkey_registered: boolean;
  config_path: string;
  config_error: string | null;
  config_warnings: string[];
}

export type UiEvent =
  | { type: "text_delta"; text: string }
  | { type: "tool_started"; call_id: string; tool: string; input: unknown }
  | {
      type: "tool_finished";
      call_id: string;
      tool: string;
      action_id: number | null;
      ok: boolean;
      detail: unknown;
    }
  | { type: "turn_complete"; stop: string }
  | { type: "turn_failed"; error: string }
  | { type: "turn_ended" }
  | { type: "turn_cancelled" };

export interface ConfirmRequest {
  id: number;
  tool: string;
  input: unknown;
  tier: Tier;
  reason: ConfirmReason;
}

export interface ActionEntry {
  id: number;
  at_unix_ms: number;
  tool: string;
  tier: Tier | null;
  args: unknown;
  gate:
    | { kind: "allowed" }
    | { kind: "confirmed" }
    | { kind: "declined" }
    | { kind: "denied"; reason: "unknown_tool" | "user_override" };
  outcome:
    | { kind: "pending" }
    | { kind: "succeeded" }
    | { kind: "failed"; error: string }
    | { kind: "undone" }
    | { kind: "not_executed" };
  undo: unknown | null;
}

export const AGENT_EVENT = "agent-event";
export const CONFIRM_EVENT = "confirm-request";
export const CONFIG_EVENT = "config-changed";
export const CONTEXT_EVENT = "context-changed";
export const MODE_EVENT = "panel-mode";

export interface ContextPreview {
  chars: number;
  preview: string;
  truncated: boolean;
}

/** What was captured on the last hotkey press. Mirrors context.rs. */
export interface ContextView {
  window: string;
  app: string | null;
  selection: ContextPreview | null;
  window_text: ContextPreview | null;
  screenshot: { data_url: string; width: number; height: number } | null;
  screenshot_note: string | null;
  capturing: boolean;
}

/** Which captured parts to send with the next message. */
export interface ContextInclude {
  selection: boolean;
  window_text: boolean;
  screenshot: boolean;
}

export const api = {
  status: () => invoke<Status>("status"),
  setApiKey: (provider: Provider, key: string) => invoke<void>("set_api_key", { provider, key }),
  clearApiKey: (provider: Provider) => invoke<void>("clear_api_key", { provider }),
  openConfig: () => invoke<void>("open_config"),
  sendMessage: (text: string, include: ContextInclude | null) =>
    invoke<void>("send_message", { text, include }),
  context: () => invoke<ContextView | null>("context"),
  dismissContext: () => invoke<void>("dismiss_context"),
  setPanelMode: (expanded: boolean) => invoke<void>("set_panel_mode", { expanded }),
  resizePanel: (height: number) => invoke<void>("resize_panel", { height }),
  hidePanel: () => invoke<void>("hide_panel"),
  insertText: (text: string) => invoke<void>("insert_text", { text }),
  cancelTurn: () => invoke<void>("cancel_turn"),
  answerConfirmation: (id: number, approved: boolean) =>
    invoke<boolean>("answer_confirmation", { id, approved }),
  actionLog: () => invoke<ActionEntry[]>("action_log"),
  undoAction: (id: number) => invoke<void>("undo_action", { id }),
  newConversation: () => invoke<void>("new_conversation"),
};

/** Subscribes to a Tauri event for the component's lifetime. */
export function useTauriEvent<T>(name: string, handler: (payload: T) => void) {
  const ref = useRef(handler);
  useLayoutEffect(() => {
    ref.current = handler;
  });
  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | undefined;
    listen<T>(name, (e) => ref.current(e.payload)).then((u) => {
      if (active) unlisten = u;
      else u();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [name]);
}

/** Tauri rejects with a plain string for command errors. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

/** Compact one-line JSON for display. */
export function compact(value: unknown, max = 120): string {
  const s = JSON.stringify(value) ?? "";
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}
