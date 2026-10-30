import { useEffect, useReducer, useRef, useState, type KeyboardEvent } from "react";
import {
  AGENT_EVENT,
  CONTEXT_EVENT,
  type ContextInclude,
  type ContextView,
  api,
  compact,
  errorText,
  useTauriEvent,
  type UiEvent,
} from "../api";
import ContextBar from "./ContextBar";

const ALL_PARTS: ContextInclude = { selection: true, window_text: true, screenshot: true };

/** Short description of what is attached, shown under the sent message. */
function attachedLabel(ctx: ContextView | null, inc: ContextInclude): string | undefined {
  if (!ctx) return undefined;
  const parts = [
    ctx.selection && inc.selection && "selection",
    ctx.window_text && inc.window_text && "window text",
    ctx.screenshot && inc.screenshot && "screenshot",
  ].filter(Boolean);
  return parts.length ? `with ${parts.join(", ")} from ${ctx.app ?? ctx.window}` : undefined;
}

type Item =
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
  | { id: number; kind: "notice"; text: string; error: boolean };

interface State {
  items: Item[];
  busy: boolean;
  nextId: number;
}

/** `Omit` applied to each member of a union separately. */
type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

type Action =
  | { type: "send"; text: string; context?: string }
  | { type: "event"; event: UiEvent }
  | { type: "send_failed"; error: string }
  | { type: "clear" };

function reducer(s: State, a: Action): State {
  const push = (item: DistributiveOmit<Item, "id">): State => ({
    ...s,
    items: [...s.items, { ...item, id: s.nextId } as Item],
    nextId: s.nextId + 1,
  });

  switch (a.type) {
    case "send":
      return { ...push({ kind: "user", text: a.text, context: a.context }), busy: true };
    case "send_failed":
      return { ...push({ kind: "notice", text: a.error, error: true }), busy: false };
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
          return {
            ...push({ kind: "notice", text: "Stopped.", error: false }),
            busy: false,
          };
        case "turn_ended":
          return { ...s, busy: false };
      }
    }
  }
}

export default function Chat() {
  const [state, dispatch] = useReducer(reducer, { items: [], busy: false, nextId: 0 });
  const [draft, setDraft] = useState("");
  const [ctx, setCtx] = useState<ContextView | null>(null);
  const [include, setInclude] = useState<ContextInclude>(ALL_PARTS);
  const endRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useTauriEvent<UiEvent>(AGENT_EVENT, (event) => dispatch({ type: "event", event }));

  const loadContext = () =>
    api.context().then((v) => {
      setCtx(v);
      // A fresh capture starts with every part attached.
      if (v?.capturing) setInclude(ALL_PARTS);
      if (v) inputRef.current?.focus();
    });
  useTauriEvent<null>(CONTEXT_EVENT, () => void loadContext());
  useEffect(() => {
    void loadContext();
    // Load once on mount; later changes arrive as events.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [state.items]);

  useEffect(() => {
    if (!state.busy) inputRef.current?.focus();
  }, [state.busy]);

  const send = () => {
    const text = draft.trim();
    if (!text || state.busy) return;
    setDraft("");
    dispatch({ type: "send", text, context: attachedLabel(ctx, include) });
    api
      .sendMessage(text, ctx ? include : null)
      .catch((e) => dispatch({ type: "send_failed", error: errorText(e) }));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    }
  };

  const newConversation = () => {
    api
      .newConversation()
      .then(() => dispatch({ type: "clear" }))
      .catch((e) => dispatch({ type: "send_failed", error: errorText(e) }));
  };

  return (
    <section className="chat">
      <div className="chat-log" aria-live="polite">
        {state.items.length === 0 && (
          <p className="empty">
            Ask DeskHand to arrange your windows, for example: “put my browser on the left and
            the editor on the right.”
          </p>
        )}
        {state.items.map((it) => {
          switch (it.kind) {
            case "user":
              return (
                <div key={it.id} className="msg-user-wrap">
                  <p className="msg msg-user">{it.text}</p>
                  {it.context && <p className="msg-context">{it.context}</p>}
                </div>
              );
            case "assistant":
              return (
                <p key={it.id} className="msg msg-assistant">
                  {it.text}
                </p>
              );
            case "tool":
              return (
                <div key={it.id} className={`tool tool-${it.state}`}>
                  <span className="tool-name">{it.tool}</span>
                  <span className="tool-args">{compact(it.input, 80)}</span>
                  {it.error && <span className="tool-error">{it.error}</span>}
                </div>
              );
            case "notice":
              return (
                <p key={it.id} className={it.error ? "notice notice-error" : "notice"}>
                  {it.text}
                </p>
              );
          }
        })}
        <div ref={endRef} />
      </div>

      <div className="composer">
        {ctx && (
          <ContextBar
            ctx={ctx}
            include={include}
            onInclude={setInclude}
            onDismiss={() => void api.dismissContext()}
          />
        )}
        <textarea
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={
            state.busy ? "Working…" : ctx ? `Ask about ${ctx.app ?? "this window"}` : "Ask DeskHand"
          }
          rows={2}
          disabled={state.busy}
          aria-label="Message"
        />
        <div className="composer-actions">
          <button type="button" className="ghost" onClick={newConversation} disabled={state.busy}>
            New
          </button>
          {state.busy ? (
            <button type="button" className="danger" onClick={() => api.cancelTurn()}>
              Stop
            </button>
          ) : (
            <button type="button" className="primary" onClick={send} disabled={!draft.trim()}>
              Send
            </button>
          )}
        </div>
      </div>
    </section>
  );
}
