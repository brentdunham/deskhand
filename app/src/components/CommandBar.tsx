import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { type ContextInclude, type ContextView, api, compact, errorText } from "../api";
import { type Item, type CompareState, QUICK_ACTIONS, codeBlocks, lastAnswer } from "../conversation";
import ContextBar from "./ContextBar";
import Markdown from "./Markdown";

/**
 * The default view: an input over the current app, the captured context, and
 * the latest answer with copy and paste-back. The window grows to fit.
 */
export default function CommandBar({
  items,
  busy,
  ctx,
  include,
  onInclude,
  onDismissContext,
  onSend,
  onExpand,
  compare,
  compareAvailable,
  onCompare,
}: {
  /** Only the current exchange: items since the bar was last opened fresh. */
  items: Item[];
  busy: boolean;
  ctx: ContextView | null;
  include: ContextInclude;
  onInclude: (next: ContextInclude) => void;
  onDismissContext: () => void;
  onSend: (text: string) => void;
  onExpand: () => void;
  /** The current comparison, shown instead of the exchange when present. */
  compare: CompareState | null;
  compareAvailable: boolean;
  onCompare: (text: string) => void;
}) {
  const [draft, setDraft] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [compareMode, setCompareMode] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const answer = lastAnswer(items);
  const blocks = codeBlocks(answer);
  const hasAnswer = !compare && !busy && answer.trim().length > 0;
  const showActions = !!ctx && !busy && !draft && items.length === 0 && !compare;
  const comparing = compareMode && compareAvailable;

  // Report content height so the native window grows and shrinks with it.
  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const report = () => void api.resizePanel(Math.ceil(el.getBoundingClientRect().height) + 2);
    report();
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!busy) inputRef.current?.focus();
  }, [busy, ctx]);

  const send = (text: string, forceCompare = false) => {
    const t = text.trim();
    if (!t || busy) return;
    setNotice(null);
    setDraft("");
    if ((comparing || forceCompare) && compareAvailable) onCompare(t);
    else onSend(t);
  };

  const copy = () => {
    void navigator.clipboard.writeText(answer).then(() => setNotice("Copied."));
  };

  const insert = (text: string) => {
    setNotice(null);
    api.insertText(text).catch((e) => setNotice(errorText(e)));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      if (draft) setDraft("");
      else void api.hidePanel();
      return;
    }
    if (e.ctrlKey && !e.altKey && /^[1-4]$/.test(e.key) && ctx && !busy) {
      e.preventDefault();
      send(QUICK_ACTIONS[Number(e.key) - 1].prompt);
      return;
    }
    if (hasAnswer && e.ctrlKey && e.key === "Enter") {
      e.preventDefault();
      insert(e.shiftKey && blocks.length ? blocks.join("\n\n") : answer);
      return;
    }
    if (hasAnswer && e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "c") {
      e.preventDefault();
      copy();
    }
  };

  const onInputKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send(draft, e.altKey);
    }
  };

  return (
    <div className="bar" ref={rootRef} onKeyDown={onKeyDown}>
      <div className="bar-input-row">
        <span
          className="bar-grip"
          title="Drag to move"
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            void api.startPanelDrag();
          }}
        >
          DH
        </span>
        <textarea
          ref={inputRef}
          className="bar-input"
          value={draft}
          rows={1}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onInputKeyDown}
          placeholder={
            busy
              ? "Working…"
              : comparing
                ? "Ask both models"
                : items.length
                  ? "Follow up"
                  : ctx
                    ? `Ask about ${ctx.app ?? "this window"}`
                    : "Ask DeskHand"
          }
          aria-label="Ask DeskHand"
        />
        {compareAvailable && !busy && (
          <button
            type="button"
            className={comparing ? "toggle on" : "toggle"}
            aria-pressed={comparing}
            title="Send to both models (Alt+Enter sends once)"
            onClick={() => {
              setCompareMode((m) => !m);
              inputRef.current?.focus();
            }}
          >
            Compare
          </button>
        )}
        {busy ? (
          <button type="button" className="danger small" onClick={() => void api.cancelTurn()}>
            Stop
          </button>
        ) : (
          <button type="button" className="icon" title="Expand" aria-label="Expand" onClick={onExpand}>
            ⤢
          </button>
        )}
      </div>

      {ctx && items.length === 0 && !compare && (
        <ContextBar ctx={ctx} include={include} onInclude={onInclude} onDismiss={onDismissContext} />
      )}

      {showActions && (
        <div className="quick-actions">
          {QUICK_ACTIONS.map((a, i) => (
            <button key={a.label} type="button" className="ghost small" onClick={() => send(a.prompt)}>
              {a.label}
              <kbd>Ctrl {i + 1}</kbd>
            </button>
          ))}
        </div>
      )}

      {compare && (
        <CompareView
          compare={compare}
          onCopy={(t) => void navigator.clipboard.writeText(t).then(() => setNotice("Copied."))}
          onInsert={insert}
        />
      )}

      {items.length > 0 && !compare && (
        <div className="bar-answer">
          {items.map((it) => {
            switch (it.kind) {
              case "user":
                return (
                  <p key={it.id} className="bar-question">
                    {it.text}
                    {it.context && <span className="msg-context"> · {it.context}</span>}
                  </p>
                );
              case "assistant":
                return <Markdown key={it.id} text={it.text} />;
              case "tool":
                return (
                  <div key={it.id} className={`tool tool-${it.state}`}>
                    <span className="tool-name">{it.tool}</span>
                    <span className="tool-args">{compact(it.input, 60)}</span>
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
                return null;
            }
          })}
          {busy && !answer && <p className="notice">Thinking…</p>}
        </div>
      )}

      {hasAnswer && (
        <div className="bar-footer">
          <button type="button" className="ghost small" onClick={copy}>
            Copy <kbd>Ctrl Shift C</kbd>
          </button>
          <button type="button" className="primary small" onClick={() => insert(answer)}>
            Insert <kbd>Ctrl Enter</kbd>
          </button>
          {blocks.length > 0 && (
            <button type="button" className="ghost small" onClick={() => insert(blocks.join("\n\n"))}>
              Insert code <kbd>Ctrl Shift Enter</kbd>
            </button>
          )}
          {notice && <span className="bar-notice">{notice}</span>}
        </div>
      )}
      {!hasAnswer && notice && <p className="notice bar-pad">{notice}</p>}
    </div>
  );
}

function CompareView({
  compare,
  onCopy,
  onInsert,
}: {
  compare: CompareState;
  onCopy: (text: string) => void;
  onInsert: (text: string) => void;
}) {
  return (
    <div className="bar-answer">
      <p className="bar-question">
        {compare.question}
        {compare.context && <span className="msg-context"> · {compare.context}</span>}
      </p>
      <div className="compare-grid">
        {compare.lanes.map((lane, i) => (
          <div key={i} className="lane">
            <div className="lane-head" title={lane.label}>
              {lane.label}
            </div>
            <div className="lane-body">
              {lane.text && <Markdown text={lane.text} />}
              {lane.error && <p className="notice notice-error">{lane.error}</p>}
              {!lane.text && !lane.error && !lane.done && <p className="notice">Thinking…</p>}
            </div>
            {lane.done && lane.text && (
              <div className="lane-actions">
                <button type="button" className="ghost small" onClick={() => onCopy(lane.text)}>
                  Copy
                </button>
                <button type="button" className="primary small" onClick={() => onInsert(lane.text)}>
                  Insert
                </button>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
