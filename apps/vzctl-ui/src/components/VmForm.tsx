import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ImageCatalogPicker } from "@/components/ImageCatalogPicker";
import {
  Card,
  FieldError,
  FormActions,
  FormField,
  FormGrid,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import { imageKeys, listImages } from "@/lib/images";
import {
  createVm,
  createdVmId,
  type CreateVmInput,
} from "@/lib/vms";

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
  const catalogQuery = useQuery({
    queryKey: imageKeys.list(),
    queryFn: listImages,
  });
  const [id, setId] = useState(initial?.id ?? "");
  const [from, setFrom] = useState(initial?.from ?? "ubuntu-latest");
  const [diskGib, setDiskGib] = useState(initial?.diskGib ?? 8);
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
      diskGib: Number(diskGib),
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
    <Card
      as="form"
      className="vm-form"
      title={mode === "replace" ? t("vmForm.replaceTitle") : t("vmForm.createTitle")}
      titleAs="h3"
      subtitle={
        mode === "replace"
          ? t("vmForm.replaceHint")
          : t("vmForm.createHint", {
              aliases: "ubuntu-latest, ubuntu-24.04, debian-12",
            })
      }
      onSubmit={(e: React.FormEvent<HTMLFormElement>) => void submit(e)}
    >
      <FormGrid>
        <FormField label={t("vmForm.id")}>
          <input
            required
            value={id}
            disabled={mode === "replace" || busy}
            onChange={(e) => setId(e.target.value)}
            placeholder="web"
          />
        </FormField>
        <ImageCatalogPicker
          value={from}
          catalog={catalogQuery.data?.catalog ?? []}
          disabled={busy}
          osLabelKey="vmForm.os"
          versionLabelKey="vmForm.version"
          onChange={setFrom}
        />
        <FormField label={t("vmForm.disk")}>
          <input
            type="number"
            min={1}
            required
            value={diskGib}
            disabled={busy}
            onChange={(e) => setDiskGib(Number(e.target.value))}
          />
        </FormField>
        <FormField label={t("vmForm.cpus")}>
          <input
            type="number"
            min={1}
            value={cpus}
            disabled={busy}
            onChange={(e) => setCpus(Number(e.target.value))}
          />
        </FormField>
        <FormField label={t("vmForm.memory")}>
          <input
            value={memory}
            disabled={busy}
            onChange={(e) => setMemory(e.target.value)}
          />
        </FormField>
        <FormField label={t("vmForm.network")}>
          <input
            value={network}
            disabled={busy}
            onChange={(e) => setNetwork(e.target.value)}
            placeholder="default"
          />
        </FormField>
        <FormField label={t("vmForm.roles")}>
          <input
            value={roles}
            disabled={busy}
            onChange={(e) => setRoles(e.target.value)}
            placeholder="docker"
          />
        </FormField>
        <FormField
          label={t("vmForm.project")}
          hint={t("vmForm.projectHint", { project: "{project}", id: "{id}" })}
        >
          <input
            value={project}
            disabled={busy}
            onChange={(e) => setProject(e.target.value)}
            placeholder="edge-dmz"
          />
        </FormField>
        <FormField label={t("vmForm.rootPassword")}>
          <input
            type="password"
            value={rootPassword}
            disabled={busy}
            onChange={(e) => setRootPassword(e.target.value)}
            autoComplete="new-password"
          />
        </FormField>
      </FormGrid>

      <FieldError message={error} />

      <FormActions
        busy={busy}
        submitLabel={
          busy
            ? mode === "replace"
              ? t("vmForm.replaceBusy")
              : t("vmForm.createBusy")
            : mode === "replace"
              ? t("vmForm.replace")
              : t("vmForm.create")
        }
        cancelLabel={t("common.cancel")}
        onCancel={onCancel}
      />
    </Card>
  );
}
