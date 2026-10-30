// Conversation state shared by the command bar and the expanded chat view.

import type { ContextInclude, ContextView, UiEvent } from "./api";

export type Item =
  | { id: number; kind: "user"; text: string; context?: string }
  | { id: number; kind: "assistant"; text: string }
  | {
      id: number;
      kind: "tool";
      callId: string;
      tool: string;
      input: unknown;
      state: "running" | "ok" | "error";
      error?: string;
    }
  | { id: number; kind: "notice"; text: string; error: boolean }
  /** Marks where a fresh conversation started. */
  | { id: number; kind: "divider" };

export interface Conversation {
  items: Item[];
  busy: boolean;
  nextId: number;
}

export const EMPTY: Conversation = { items: [], busy: false, nextId: 0 };

export type Action =
  | { type: "send"; text: string; context?: string }
  | { type: "event"; event: UiEvent }
  | { type: "send_failed"; error: string }
  | { type: "divider" }
  | { type: "clear" };

/** `Omit` applied to each member of a union separately. */
type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

export function reducer(s: Conversation, a: Action): Conversation {
  const push = (item: DistributiveOmit<Item, "id">): Conversation => ({
    ...s,
    items: [...s.items, { ...item, id: s.nextId } as Item],
    nextId: s.nextId + 1,
  });

  switch (a.type) {
    case "send":
      return { ...push({ kind: "user", text: a.text, context: a.context }), busy: true };
    case "send_failed":
      return { ...push({ kind: "notice", text: a.error, error: true }), busy: false };
    case "divider":
      return push({ kind: "divider" });
    case "clear":
      return { ...s, items: [] };
    case "event": {
      const e = a.event;
      switch (e.type) {
        case "text_delta": {
          const last = s.items[s.items.length - 1];
          if (last?.kind === "assistant") {
            const items = s.items.slice(0, -1);
            items.push({ ...last, text: last.text + e.text });
            return { ...s, items };
          }
          return push({ kind: "assistant", text: e.text });
        }
        case "tool_started":
          return push({
            kind: "tool",
            callId: e.call_id,
            tool: e.tool,
            input: e.input,
            state: "running",
          });
        case "tool_finished":
          return {
            ...s,
            items: s.items.map((it) =>
              it.kind === "tool" && it.callId === e.call_id
                ? {
                    ...it,
                    state: e.ok ? "ok" : "error",
                    error: e.ok ? undefined : String(e.detail),
                  }
                : it,
            ),
          };
        case "turn_complete":
          return e.stop === "max_tokens"
            ? push({ kind: "notice", text: "The reply hit the length limit.", error: false })
            : s;
        case "turn_failed":
          return push({ kind: "notice", text: e.error, error: true });
        case "turn_cancelled":
          return { ...push({ kind: "notice", text: "Stopped.", error: false }), busy: false };
        case "turn_ended":
          return { ...s, busy: false };
      }
    }
  }
}

export const ALL_PARTS: ContextInclude = { selection: true, window_text: true, screenshot: true };

/** Short description of what is attached, shown under the sent message. */
export function attachedLabel(ctx: ContextView | null, inc: ContextInclude): string | undefined {
  if (!ctx) return undefined;
  const parts = [
    ctx.selection && inc.selection && "selection",
    ctx.window_text && inc.window_text && "window text",
    ctx.screenshot && inc.screenshot && "screenshot",
  ].filter(Boolean);
  return parts.length ? `with ${parts.join(", ")} from ${ctx.app ?? ctx.window}` : undefined;
}

/** All assistant text after the last user message. */
export function lastAnswer(items: Item[]): string {
  const start = items.map((i) => i.kind).lastIndexOf("user");
  return items
    .slice(start + 1)
    .filter((i): i is Extract<Item, { kind: "assistant" }> => i.kind === "assistant")
    .map((i) => i.text)
    .join("\n\n");
}

/** The contents of fenced code blocks, in order. */
export function codeBlocks(markdown: string): string[] {
  const out: string[] = [];
  const re = /```[^\n]*\n([\s\S]*?)```/g;
  for (let m = re.exec(markdown); m; m = re.exec(markdown)) {
    out.push(m[1].replace(/\n$/, ""));
  }
  return out;
}

/** Quick actions offered when context is attached. */
export const QUICK_ACTIONS: { label: string; prompt: string }[] = [
  { label: "Explain", prompt: "Explain this." },
  { label: "Summarize", prompt: "Summarize this." },
  { label: "Fix", prompt: "Find what's wrong here and give the fix." },
  { label: "Rewrite", prompt: "Rewrite this to be clearer. Reply with only the rewritten text." },
];
