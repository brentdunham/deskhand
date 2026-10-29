import { useCallback, useEffect, useState } from "react";
import { type ActionEntry, api, compact, errorText } from "../api";

function gateLabel(e: ActionEntry): string {
  switch (e.gate.kind) {
    case "allowed":
      return "";
    case "confirmed":
      return "approved";
    case "declined":
      return "declined";
    case "denied":
      return e.gate.reason === "unknown_tool" ? "blocked: unknown tool" : "blocked: disabled";
  }
}

function outcomeLabel(e: ActionEntry): string {
  switch (e.outcome.kind) {
    case "pending":
      return "running";
    case "succeeded":
      return "done";
    case "failed":
      return "failed";
    case "undone":
      return "undone";
    case "not_executed":
      return "not run";
  }
}

/** Newest first. `version` changes whenever a tool finishes, to refresh. */
export default function ActionLog({ version }: { version: number }) {
  const [entries, setEntries] = useState<ActionEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .actionLog()
      .then((e) => setEntries([...e].reverse()))
      .catch((e) => setError(errorText(e)));
  }, []);

  useEffect(load, [load, version]);

  const undo = (id: number) => {
    setError(null);
    api
      .undoAction(id)
      .then(load)
      .catch((e) => setError(errorText(e)));
  };

  return (
    <section className="log">
      {error && <p className="notice notice-error">{error}</p>}
      {entries.length === 0 && <p className="empty">No actions yet.</p>}
      <ul>
        {entries.map((e) => {
          const gate = gateLabel(e);
          return (
            <li key={e.id} className={`entry outcome-${e.outcome.kind}`}>
              <div className="entry-head">
                <span className="tool-name">{e.tool}</span>
                <span className="entry-status">
                  {gate && <span className="badge">{gate}</span>}
                  <span className="badge">{outcomeLabel(e)}</span>
                </span>
                <time className="muted">{new Date(e.at_unix_ms).toLocaleTimeString()}</time>
              </div>
              <div className="tool-args">{compact(e.args, 100)}</div>
              {e.outcome.kind === "failed" && <div className="tool-error">{e.outcome.error}</div>}
              {e.outcome.kind === "succeeded" && e.undo != null && (
                <button type="button" className="ghost small" onClick={() => undo(e.id)}>
                  Undo
                </button>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
