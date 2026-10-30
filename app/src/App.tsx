import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import {
  AGENT_EVENT,
  CONFIG_EVENT,
  CONFIRM_EVENT,
  CONTEXT_EVENT,
  type ContextInclude,
  type ContextView,
  MODE_EVENT,
  type Status,
  type UiEvent,
  api,
  errorText,
  useTauriEvent,
} from "./api";
import ActionLog from "./components/ActionLog";
import Chat from "./components/Chat";
import CommandBar from "./components/CommandBar";
import ConfirmDialog from "./components/ConfirmDialog";
import Settings, { Onboarding } from "./components/Settings";
import { ALL_PARTS, EMPTY, attachedLabel, reducer } from "./conversation";

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
  const [expanded, setExpanded] = useState(false);

  const [conv, dispatch] = useReducer(reducer, EMPTY);
  const convRef = useRef(conv);
  convRef.current = conv;

  const [ctx, setCtx] = useState<ContextView | null>(null);
  const [include, setInclude] = useState<ContextInclude>(ALL_PARTS);
  // The command bar shows items from here on: just the current exchange.
  const [barStart, setBarStart] = useState(0);
  // The next question starts a new model conversation (set on each capture).
  const freshRef = useRef(true);

  const refresh = useCallback(() => {
    api
      .status()
      .then((s) => {
        setStatus(s);
        setError(null);
      })
      .catch((e) => setError(errorText(e)));
  }, []);

  const loadContext = useCallback(() => {
    void api.context().then((v) => {
      setCtx(v);
      if (v?.capturing) {
        // A new hotkey press: fresh question, every part attached.
        setInclude(ALL_PARTS);
        if (!convRef.current.busy) {
          freshRef.current = true;
          setBarStart(convRef.current.items.length);
        }
      }
    });
  }, []);

  useEffect(refresh, [refresh]);
  useEffect(loadContext, [loadContext]);

  useTauriEvent<UiEvent>(AGENT_EVENT, (event) => {
    dispatch({ type: "event", event });
    if (event.type === "tool_finished") setLogVersion((v) => v + 1);
  });
  useTauriEvent<null>(CONFIG_EVENT, refresh);
  useTauriEvent<null>(CONTEXT_EVENT, loadContext);
  useTauriEvent<boolean>(MODE_EVENT, setExpanded);
  // A confirmation needs room; the bar expands to show it.
  useTauriEvent<unknown>(CONFIRM_EVENT, () => {
    if (!expanded) void api.setPanelMode(true);
  });

  useEffect(() => {
    if (status?.needs_key) void api.setPanelMode(true);
  }, [status?.needs_key]);

  const send = async (text: string) => {
    const label = attachedLabel(ctx, include);
    const inc = ctx ? include : null;
    if (freshRef.current && convRef.current.items.length > 0) {
      try {
        await api.newConversation();
      } catch {
        // Busy elsewhere; continue the current conversation instead.
      }
      dispatch({ type: "divider" });
      setBarStart(convRef.current.items.length + 1);
    }
    freshRef.current = false;
    dispatch({ type: "send", text, context: label });
    api
      .sendMessage(text, inc)
      .catch((e) => dispatch({ type: "send_failed", error: errorText(e) }));
  };

  const newConversation = () => {
    api
      .newConversation()
      .then(() => {
        dispatch({ type: "clear" });
        setBarStart(0);
      })
      .catch((e) => dispatch({ type: "send_failed", error: errorText(e) }));
  };

  if (error) {
    return (
      <main className="onboarding">
        <p className="notice notice-error">{error}</p>
      </main>
    );
  }
  if (!status) return <main className="onboarding muted">Starting…</main>;
  if (status.needs_key) return <Onboarding onSaved={refresh} />;

  if (!expanded) {
    return (
      <>
        <CommandBar
          items={conv.items.slice(barStart).filter((i) => i.kind !== "divider")}
          busy={conv.busy}
          ctx={ctx}
          include={include}
          onInclude={setInclude}
          onDismissContext={() => void api.dismissContext()}
          onSend={(t) => void send(t)}
          onExpand={() => void api.setPanelMode(true)}
        />
        {/* Same position in both modes, so a pending confirmation survives
            the switch to the expanded view. */}
        <ConfirmDialog />
      </>
    );
  }

  return (
    <>
    <div className="app">
      <header className="topbar" data-tauri-drag-region>
        <span className="brand" data-tauri-drag-region>
          DeskHand
        </span>
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
          <button
            type="button"
            className="icon"
            title="Collapse to the command bar"
            aria-label="Collapse"
            onClick={() => void api.setPanelMode(false)}
          >
            ⤡
          </button>
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

      <div className="view" hidden={tab !== "chat"}>
        <Chat
          conversation={conv}
          ctx={ctx}
          include={include}
          onInclude={setInclude}
          onDismissContext={() => void api.dismissContext()}
          onSend={(t) => void send(t)}
          onNew={newConversation}
        />
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
    </div>
    <ConfirmDialog />
    </>
  );
}
