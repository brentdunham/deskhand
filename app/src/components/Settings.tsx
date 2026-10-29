import { useState, type FormEvent } from "react";
import { api, errorText, type Status } from "../api";

function KeyForm({ onSaved, submitLabel }: { onSaved: () => void; submitLabel: string }) {
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    setError(null);
    api
      .setApiKey(key)
      .then(() => {
        setKey("");
        onSaved();
      })
      .catch((err) => setError(errorText(err)))
      .finally(() => setSaving(false));
  };

  return (
    <form className="key-form" onSubmit={submit}>
      <label htmlFor="api-key">Anthropic API key</label>
      <input
        id="api-key"
        type="password"
        autoComplete="off"
        spellCheck={false}
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="sk-ant-…"
      />
      {error && <p className="notice notice-error">{error}</p>}
      <button type="submit" className="primary" disabled={!key.trim() || saving}>
        {submitLabel}
      </button>
    </form>
  );
}

const KEY_NOTE =
  "Stored in Windows Credential Manager and sent only to api.anthropic.com. DeskHand can’t display it again.";

export function Onboarding({ onSaved }: { onSaved: () => void }) {
  return (
    <section className="onboarding">
      <h1>DeskHand</h1>
      <p>DeskHand needs an Anthropic API key to talk to Claude.</p>
      <KeyForm onSaved={onSaved} submitLabel="Save key" />
      <p className="muted">{KEY_NOTE}</p>
    </section>
  );
}

export default function Settings({ status, onChange }: { status: Status; onChange: () => void }) {
  const [error, setError] = useState<string | null>(null);

  const remove = () => {
    setError(null);
    api
      .clearApiKey()
      .then(onChange)
      .catch((e) => setError(errorText(e)));
  };

  return (
    <section className="settings">
      <dl>
        <dt>Model</dt>
        <dd>
          <code>{status.model}</code>
        </dd>
        <dt>Show or hide</dt>
        <dd>
          {status.hotkey}
          {!status.hotkey_registered && <span className="badge warn">unavailable</span>}
        </dd>
      </dl>

      <h2>API key</h2>
      <KeyForm onSaved={onChange} submitLabel="Replace key" />
      <p className="muted">{KEY_NOTE}</p>
      {error && <p className="notice notice-error">{error}</p>}
      <button type="button" className="danger" onClick={remove}>
        Remove key
      </button>
    </section>
  );
}
