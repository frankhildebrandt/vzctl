import { useState } from "react";
import { pickDirectory } from "@/lib/dialogs";
import { mountVm } from "@/lib/vms";

export function VmMountForm({
  vmId,
  onDone,
  onCancel,
}: {
  vmId: string;
  onDone: () => void | Promise<void>;
  onCancel: () => void;
}) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [tag, setTag] = useState("");
  const [readOnly, setReadOnly] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function chooseSource() {
    const path = await pickDirectory("Host-Verzeichnis für Mount");
    if (path) setSource(path);
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await mountVm({
        id: vmId,
        source: source.trim(),
        target: target.trim(),
        tag: tag.trim() || undefined,
        readOnly,
      });
      await onDone();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="card vm-form" onSubmit={(e) => void submit(e)}>
      <h3>Mount hinzufügen</h3>
      <p className="muted">virtiofs Share (Host → Guest).</p>
      <div className="form-grid">
        <label className="form-span-2">
          Source (Host)
          <div className="row" style={{ gap: "0.5rem" }}>
            <input
              required
              value={source}
              disabled={busy}
              onChange={(e) => setSource(e.target.value)}
              placeholder="/Users/…"
              style={{ flex: 1 }}
            />
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => void chooseSource()}
            >
              Wählen…
            </button>
          </div>
        </label>
        <label>
          Target (Guest)
          <input
            required
            value={target}
            disabled={busy}
            onChange={(e) => setTarget(e.target.value)}
            placeholder="/srv/app"
          />
        </label>
        <label>
          Tag / Name
          <input
            value={tag}
            disabled={busy}
            onChange={(e) => setTag(e.target.value)}
            placeholder="app"
          />
        </label>
        <label className="form-check">
          <input
            type="checkbox"
            checked={readOnly}
            disabled={busy}
            onChange={(e) => setReadOnly(e.target.checked)}
          />
          Read-only
        </label>
      </div>
      {error ? <p className="form-error">{error}</p> : null}
      <div className="row" style={{ gap: "0.5rem" }}>
        <button type="submit" disabled={busy}>
          {busy ? "Mount…" : "Mount"}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy}
          onClick={onCancel}
        >
          Abbrechen
        </button>
      </div>
    </form>
  );
}
