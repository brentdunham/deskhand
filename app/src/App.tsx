import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function App() {
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("app_version")
      .then(setVersion)
      .catch((e: unknown) => setError(String(e)));
  }, []);

  return (
    <main className="shell">
      <h1>DeskHand</h1>
      {error ? (
        <p className="status status-error">IPC error: {error}</p>
      ) : (
        <p className="status">Core {version ? `v${version}` : "connecting..."}</p>
      )}
    </main>
  );
}
