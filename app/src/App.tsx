import { useCallback, useEffect, useState } from "react";
import {
  AGENT_EVENT,
  CONFIG_EVENT,
  api,
  errorText,
  type Status,
  type UiEvent,
  useTauriEvent,
} from "./api";
import ActionLog from "./components/ActionLog";
import Chat from "./components/Chat";
import ConfirmDialog from "./components/ConfirmDialog";
import Settings, { Onboarding } from "./components/Settings";

type Tab = "chat" | "activity" | "settings";

const TABS: { id: Tab; label: string }[] = [
  { id: "chat", label: "Chat" },
  { id: "activity", label: "Activity" },
  { id: "settings", label: "Settings" },
];

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("chat");
  const [logVersion, setLogVersion] = useState(0);

  const refresh = useCallback(() => {
    api
      .status()
      .then((s) => {
        setStatus(s);
        setError(null);
      })
      .catch((e) => setError(errorText(e)));
  }, []);

  useEffect(refresh, [refresh]);

  useTauriEvent<UiEvent>(AGENT_EVENT, (e) => {
    if (e.type === "tool_finished") setLogVersion((v) => v + 1);
  });
  useTauriEvent<null>(CONFIG_EVENT, refresh);

  if (error) {
    return (
      <main className="onboarding">
        <p className="notice notice-error">{error}</p>
      </main>
    );
  }
  if (!status) return <main className="onboarding muted">Starting…</main>;
  if (status.needs_key) return <Onboarding onSaved={refresh} />;

  return (
    <div className="app">
      <header className="topbar">
        <span className="brand">DeskHand</span>
        <nav className="tabs" aria-label="Views">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              className={tab === t.id ? "tab active" : "tab"}
              aria-current={tab === t.id ? "page" : undefined}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      {!status.hotkey_registered && (
        <p className="banner">
          {status.hotkey} is in use by another app. Open DeskHand from the tray icon, or set a
          different hotkey in the config file.
        </p>
      )}
      {status.config_error && (
        <div className="banner banner-error" role="alert">
          <p>The config file has an error. The previous settings are still in effect.</p>
          <pre className="config-error">{status.config_error}</pre>
        </div>
      )}
      {status.config_warnings.map((w) => (
        <p key={w} className="banner">
          {w}
        </p>
      ))}

      {/* Chat stays mounted so it keeps receiving events on other tabs. */}
      <div className="view" hidden={tab !== "chat"}>
        <Chat />
      </div>
      {tab === "activity" && (
        <div className="view">
          <ActionLog version={logVersion} />
        </div>
      )}
      {tab === "settings" && (
        <div className="view">
          <Settings status={status} onChange={refresh} />
        </div>
      )}

      <ConfirmDialog />
    </div>
  );
}
