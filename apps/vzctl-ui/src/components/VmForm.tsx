import { useState } from "react";
import { useT } from "@/lib/i18n";
import {
  createVm,
  createdVmId,
  type CreateVmInput,
} from "@/lib/vms";
import { IMAGE_ALIAS_HINTS } from "@/lib/images";

type Mode = "create" | "replace";

export function VmForm({
  mode,
  initial,
  onDone,
  onCancel,
  onSubmitReplace,
}: {
  mode: Mode;
  initial?: Partial<CreateVmInput>;
  onDone: (id: string) => void | Promise<void>;
  onCancel: () => void;
  onSubmitReplace?: (input: CreateVmInput) => Promise<void>;
}) {
  const t = useT();
  const [id, setId] = useState(initial?.id ?? "");
  const [from, setFrom] = useState(initial?.from ?? "ubuntu-latest");
  const [dataDiskGib, setDataDiskGib] = useState(initial?.dataDiskGib ?? 8);
  const [cpus, setCpus] = useState(initial?.cpus ?? 2);
  const [memory, setMemory] = useState(initial?.memory ?? "1024");
  const [network, setNetwork] = useState(initial?.network ?? "");
  const [roles, setRoles] = useState((initial?.roles ?? []).join(","));
  const [rootPassword, setRootPassword] = useState(initial?.rootPassword ?? "");
  const [project, setProject] = useState(initial?.project ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setBusy(true);
    const input: CreateVmInput = {
      id: id.trim(),
      from: from.trim(),
      dataDiskGib: Number(dataDiskGib),
      cpus: Number(cpus),
      memory: memory.trim() || undefined,
      network: network.trim() || undefined,
      project: project.trim() || undefined,
      rootPassword: rootPassword || undefined,
      roles: roles
        .split(",")
        .map((role) => role.trim())
        .filter(Boolean),
    };
    try {
      if (mode === "replace" && onSubmitReplace) {
        await onSubmitReplace(input);
        // Parent owns confirmation + completion for replace.
        return;
      }
      const envelope = await createVm(input);
      await onDone(createdVmId(envelope, input.id));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="card vm-form" onSubmit={(e) => void submit(e)}>
      <h3>{mode === "replace" ? t("vmForm.replaceTitle") : t("vmForm.createTitle")}</h3>
      <p className="muted">
        {mode === "replace"
          ? t("vmForm.replaceHint")
          : t("vmForm.createHint", {
              aliases: IMAGE_ALIAS_HINTS.slice(0, 4).join(", "),
            })}
      </p>

      <div className="form-grid">
        <label>
          {t("vmForm.id")}
          <input
            required
            value={id}
            disabled={mode === "replace" || busy}
            onChange={(e) => setId(e.target.value)}
            placeholder="web"
          />
        </label>
        <label>
          {t("vmForm.from")}
          <input
            required
            value={from}
            disabled={busy}
            onChange={(e) => setFrom(e.target.value)}
            placeholder="ubuntu-latest"
            list="image-aliases"
          />
          <datalist id="image-aliases">
            {IMAGE_ALIAS_HINTS.map((alias) => (
              <option key={alias} value={alias} />
            ))}
          </datalist>
        </label>
        <label>
          {t("vmForm.dataDisk")}
          <input
            type="number"
            min={1}
            required
            value={dataDiskGib}
            disabled={busy}
            onChange={(e) => setDataDiskGib(Number(e.target.value))}
          />
        </label>
        <label>
          {t("vmForm.cpus")}
          <input
            type="number"
            min={1}
            value={cpus}
            disabled={busy}
            onChange={(e) => setCpus(Number(e.target.value))}
          />
        </label>
        <label>
          {t("vmForm.memory")}
          <input
            value={memory}
            disabled={busy}
            onChange={(e) => setMemory(e.target.value)}
          />
        </label>
        <label>
          {t("vmForm.network")}
          <input
            value={network}
            disabled={busy}
            onChange={(e) => setNetwork(e.target.value)}
            placeholder="default"
          />
        </label>
        <label>
          {t("vmForm.roles")}
          <input
            value={roles}
            disabled={busy}
            onChange={(e) => setRoles(e.target.value)}
            placeholder="docker"
          />
        </label>
        <label>
          {t("vmForm.project")}
          <input
            value={project}
            disabled={busy}
            onChange={(e) => setProject(e.target.value)}
            placeholder="edge-dmz"
          />
          <span className="muted">
            {t("vmForm.projectHint", { project: "{project}", id: "{id}" })}
          </span>
        </label>
        <label>
          {t("vmForm.rootPassword")}
          <input
            type="password"
            value={rootPassword}
            disabled={busy}
            onChange={(e) => setRootPassword(e.target.value)}
            autoComplete="new-password"
          />
        </label>
      </div>

      {error ? <p className="form-error">{error}</p> : null}

      <div className="row" style={{ gap: "0.5rem" }}>
        <button type="submit" disabled={busy}>
          {busy
            ? mode === "replace"
              ? t("vmForm.replaceBusy")
              : t("vmForm.createBusy")
            : mode === "replace"
              ? t("vmForm.replace")
              : t("vmForm.create")}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy}
          onClick={onCancel}
        >
          {t("common.cancel")}
        </button>
      </div>
    </form>
  );
}
