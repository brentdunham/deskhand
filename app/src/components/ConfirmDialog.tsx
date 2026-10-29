import { useState } from "react";
import {
  AGENT_EVENT,
  CONFIRM_EVENT,
  api,
  type ConfirmReason,
  type ConfirmRequest,
  type UiEvent,
  useTauriEvent,
} from "../api";

function reasonText(reason: ConfirmReason): string {
  switch (reason.kind) {
    case "irreversible":
      return "This action can’t be undone.";
    case "user_override":
      return "You’ve set this tool to ask before it runs.";
    case "tainted_context":
      return `This conversation includes content from outside DeskHand (${reason.sources.join(
        ", ",
      )}), which could be steering this request. Approve only if you asked for this.`;
  }
}

/**
 * Shows one pending confirmation at a time. Decline has initial focus, so
 * pressing Enter on an unexpected dialog never approves an action.
 */
export default function ConfirmDialog() {
  const [queue, setQueue] = useState<ConfirmRequest[]>([]);

  useTauriEvent<ConfirmRequest>(CONFIRM_EVENT, (req) => setQueue((q) => [...q, req]));
  // The backend drops pending requests when a turn is cancelled.
  useTauriEvent<UiEvent>(AGENT_EVENT, (e) => {
    if (e.type === "turn_cancelled") setQueue([]);
  });

  const current = queue[0];
  if (!current) return null;

  const answer = (approved: boolean) => {
    setQueue((q) => q.slice(1));
    api.answerConfirmation(current.id, approved);
  };

  return (
    <div className="overlay">
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        aria-describedby="confirm-reason"
      >
        <h2 id="confirm-title">
          Allow <code>{current.tool}</code>?
        </h2>
        <p id="confirm-reason">{reasonText(current.reason)}</p>
        <pre className="args">{JSON.stringify(current.input, null, 2)}</pre>
        <div className="dialog-actions">
          <button type="button" className="ghost" autoFocus onClick={() => answer(false)}>
            Decline
          </button>
          <button type="button" className="primary" onClick={() => answer(true)}>
            Allow
          </button>
        </div>
      </div>
    </div>
  );
}
