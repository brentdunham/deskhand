import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { type ContextInclude, type ContextView, api, compact } from "../api";
import type { Conversation } from "../conversation";
import ContextBar from "./ContextBar";
import Markdown from "./Markdown";

/** The expanded view's full conversation history. State lives in App. */
export default function Chat({
  conversation,
  ctx,
  include,
  onInclude,
  onDismissContext,
  onSend,
  onNew,
}: {
  conversation: Conversation;
  ctx: ContextView | null;
  include: ContextInclude;
  onInclude: (next: ContextInclude) => void;
  onDismissContext: () => void;
  onSend: (text: string) => void;
  onNew: () => void;
}) {
  const [draft, setDraft] = useState("");
  const endRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const { items, busy } = conversation;

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [items]);

  useEffect(() => {
    if (!busy) inputRef.current?.focus();
  }, [busy]);

  const send = () => {
    const text = draft.trim();
    if (!text || busy) return;
    setDraft("");
    onSend(text);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    }
  };

  return (
    <section className="chat">
      <div className="chat-log" aria-live="polite">
        {items.length === 0 && (
          <p className="empty">
            Press the hotkey in any app to ask about what you're looking at, or ask here.
          </p>
        )}
        {items.map((it) => {
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
                <div key={it.id} className="msg msg-assistant">
                  <Markdown text={it.text} />
                </div>
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
            case "divider":
              return (
                <p key={it.id} className="divider">
                  New conversation
                </p>
              );
          }
        })}
        <div ref={endRef} />
      </div>

      <div className="composer">
        {ctx && (
          <ContextBar ctx={ctx} include={include} onInclude={onInclude} onDismiss={onDismissContext} />
        )}
        <textarea
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={busy ? "Working…" : ctx ? `Ask about ${ctx.app ?? "this window"}` : "Ask DeskHand"}
          rows={2}
          disabled={busy}
          aria-label="Message"
        />
        <div className="composer-actions">
          <button type="button" className="ghost" onClick={onNew} disabled={busy}>
            New
          </button>
          {busy ? (
            <button type="button" className="danger" onClick={() => void api.cancelTurn()}>
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
