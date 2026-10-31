import { useState, type FormEvent } from "react";
import { api, errorText, type Provider, type Status } from "../api";

const PROVIDER_LABEL: Record<Provider, string> = {
  anthropic: "Anthropic",
  openai_compatible: "OpenAI-compatible",
};

function KeyForm({
  provider,
  onSaved,
  submitLabel,
}: {
  provider: Provider;
  onSaved: () => void;
  submitLabel: string;
}) {
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const id = `api-key-${provider}`;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    setError(null);
    api
      .setApiKey(provider, key)
      .then(() => {
        setKey("");
        onSaved();
      })
      .catch((err) => setError(errorText(err)))
      .finally(() => setSaving(false));
  };

  return (
    <form className="key-form" onSubmit={submit}>
      <label htmlFor={id} className="visually-hidden">
        {PROVIDER_LABEL[provider]} API key
      </label>
      <input
        id={id}
        type="password"
        autoComplete="off"
        spellCheck={false}
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder={provider === "anthropic" ? "sk-ant-…" : "API key"}
      />
      {error && <p className="notice notice-error">{error}</p>}
      <button type="submit" className="primary" disabled={!key.trim() || saving}>
        {submitLabel}
      </button>
    </form>
  );
}

function keyNote(destination: string): string {
  return `Stored in Windows Credential Manager and sent only to ${destination}. DeskHand can’t display it again.`;
}

export function Onboarding({ onSaved }: { onSaved: () => void }) {
  return (
    <section className="onboarding">
      <h1>DeskHand</h1>
      <p>DeskHand needs an Anthropic API key to talk to Claude.</p>
      <KeyForm provider="anthropic" onSaved={onSaved} submitLabel="Save key" />
      <p className="muted">{keyNote("api.anthropic.com")}</p>
      <p className="muted">
        To use a local model instead, set <code>kind = "openai_compatible"</code> in the config
        file.
      </p>
      <button type="button" className="ghost" onClick={() => api.openConfig()}>
        Open config file
      </button>
    </section>
  );
}

function KeySection({
  provider,
  saved,
  optional,
  destination,
  onChange,
}: {
  provider: Provider;
  saved: boolean;
  optional: boolean;
  destination: string;
  onChange: () => void;
}) {
  const [error, setError] = useState<string | null>(null);

  const remove = () => {
    setError(null);
    api
      .clearApiKey(provider)
      .then(onChange)
      .catch((e) => setError(errorText(e)));
  };

  return (
    <>
      <h2>
        {PROVIDER_LABEL[provider]} API key{optional ? " (optional)" : ""}
        <span className="badge">{saved ? "saved" : "not set"}</span>
      </h2>
      <KeyForm provider={provider} onSaved={onChange} submitLabel={saved ? "Replace key" : "Save key"} />
      <p className="muted">{keyNote(destination)}</p>
      {error && <p className="notice notice-error">{error}</p>}
      {saved && (
        <button type="button" className="danger" onClick={remove}>
          Remove key
        </button>
      )}
    </>
  );
}

export default function Settings({ status, onChange }: { status: Status; onChange: () => void }) {
  const [openError, setOpenError] = useState<string | null>(null);
  const openConfig = () => {
    setOpenError(null);
    api.openConfig().catch((e) => setOpenError(errorText(e)));
  };

  const anthropicDestination =
    status.provider === "anthropic" && status.base_url ? status.base_url : "api.anthropic.com";
  const usesAnthropic = status.provider === "anthropic" || status.compare?.kind === "anthropic";
  const usesOpenAi =
    status.provider === "openai_compatible" || status.compare?.kind === "openai_compatible";
  // One key slot serves every OpenAI-compatible endpoint in use.
  const openAiDestination =
    [
      status.provider === "openai_compatible" ? status.base_url : null,
      status.compare?.kind === "openai_compatible" ? status.compare.base_url : null,
    ]
      .filter((u): u is string => !!u)
      .join(" and ") || "the configured endpoint";

  return (
    <section className="settings">
      <dl>
        <dt>Provider</dt>
        <dd>{PROVIDER_LABEL[status.provider]}</dd>
        <dt>Model</dt>
        <dd>
          <code>{status.model}</code>
        </dd>
        {status.base_url && (
          <>
            <dt>Endpoint</dt>
            <dd>
              <code>{status.base_url}</code>
            </dd>
          </>
        )}
        <dt>Show or hide</dt>
        <dd>
          {status.hotkey}
          {!status.hotkey_registered && <span className="badge warn">unavailable</span>}
        </dd>
        <dt>Compare with</dt>
        <dd>
          {status.compare ? (
            <code>{status.compare.labels[1]}</code>
          ) : (
            <span className="muted">not set (add [compare] to the config file)</span>
          )}
        </dd>
      </dl>

      <h2>MCP servers</h2>
      {status.mcp.length === 0 ? (
        <p className="muted">
          None configured. Add a <code>[mcp.servers.&lt;name&gt;]</code> section to the config file.
        </p>
      ) : (
        <ul className="mcp-list">
          {status.mcp.map((s) => (
            <li key={s.name} className="mcp-server">
              <div className="mcp-head">
                <code>{s.name}</code>
                <span className={s.state === "failed" ? "badge warn" : "badge"}>
                  {s.state === "ready"
                    ? `${s.tools} tool${s.tools === 1 ? "" : "s"}`
                    : s.state}
                </span>
                <span className="muted mcp-command">{s.command}</span>
              </div>
              {s.state === "failed" && <p className="notice notice-error">{s.error}</p>}
              {s.state === "ready" && s.skipped.length > 0 && (
                <p className="muted">Skipped (name not usable): {s.skipped.join(", ")}</p>
              )}
            </li>
          ))}
        </ul>
      )}

      <h2>Config file</h2>
      <p className="muted path">{status.config_path}</p>
      <p className="muted">Changes apply as soon as you save the file.</p>
      {openError && <p className="notice notice-error">{openError}</p>}
      <button type="button" className="ghost" onClick={openConfig}>
        Open in Notepad
      </button>

      {usesAnthropic && (
        <KeySection
          provider="anthropic"
          saved={status.has_anthropic_key}
          optional={false}
          destination={anthropicDestination}
          onChange={onChange}
        />
      )}
      {usesOpenAi && (
        <KeySection
          provider="openai_compatible"
          saved={status.has_openai_key}
          optional
          destination={openAiDestination}
          onChange={onChange}
        />
      )}
    </section>
  );
}
