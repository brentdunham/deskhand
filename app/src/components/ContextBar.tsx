import type { ContextInclude, ContextView } from "../api";

function count(n: number, truncated: boolean): string {
  const k = n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
  return `${k}${truncated ? "+" : ""} chars`;
}

/**
 * What the hotkey captured, as removable chips. Everything shown here is sent
 * with the next message; nothing else is.
 */
export default function ContextBar({
  ctx,
  include,
  onInclude,
  onDismiss,
}: {
  ctx: ContextView;
  include: ContextInclude;
  onInclude: (next: ContextInclude) => void;
  onDismiss: () => void;
}) {
  const drop = (part: keyof ContextInclude) => onInclude({ ...include, [part]: false });
  const nothingAttached =
    !ctx.capturing &&
    !(ctx.selection && include.selection) &&
    !(ctx.window_text && include.window_text) &&
    !(ctx.screenshot && include.screenshot);

  return (
    <div className="context-bar" aria-label="Context to send">
      <div className="context-head">
        <span className="context-window" title={ctx.window}>
          {ctx.app && <span className="context-app">{ctx.app}</span>}
          {ctx.window}
        </span>
        <button type="button" className="icon" aria-label="Remove all context" onClick={onDismiss}>
          ×
        </button>
      </div>
      <div className="chips">
        {ctx.selection && include.selection && (
          <span className="chip" title={ctx.selection.preview}>
            Selection · {count(ctx.selection.chars, false)}
            <button type="button" aria-label="Remove selection" onClick={() => drop("selection")}>
              ×
            </button>
          </span>
        )}
        {ctx.window_text && include.window_text && (
          <span className="chip" title={ctx.window_text.preview}>
            Window text · {count(ctx.window_text.chars, ctx.window_text.truncated)}
            <button type="button" aria-label="Remove window text" onClick={() => drop("window_text")}>
              ×
            </button>
          </span>
        )}
        {ctx.screenshot && include.screenshot && (
          <span className="chip chip-image">
            <img
              src={ctx.screenshot.data_url}
              alt="Screenshot of the window"
              width={ctx.screenshot.width}
              height={ctx.screenshot.height}
            />
            <button type="button" aria-label="Remove screenshot" onClick={() => drop("screenshot")}>
              ×
            </button>
          </span>
        )}
        {ctx.capturing && <span className="chip chip-muted">Capturing…</span>}
        {!ctx.capturing && !ctx.screenshot && ctx.screenshot_note && (
          <span className="chip chip-muted">No screenshot: {ctx.screenshot_note}</span>
        )}
        {nothingAttached && <span className="chip chip-muted">Nothing will be attached</span>}
      </div>
    </div>
  );
}
