import { useState, type FormEvent } from "react";
import {
  ActionRow,
  Button,
  Card,
  FieldError,
  FormActions,
  FormCheck,
  FormField,
  FormGrid,
} from "@/components/ui";
import { pickDirectory } from "@/lib/dialogs";
import { useT } from "@/lib/i18n";
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
  const t = useT();
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [tag, setTag] = useState("");
  const [readOnly, setReadOnly] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function chooseSource() {
    const path = await pickDirectory(t("mount.pickTitle"));
    if (path) setSource(path);
  }

  async function submit(event: FormEvent) {
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
    <Card
      as="form"
      className="vm-form"
      title={t("mount.title")}
      titleAs="h3"
      subtitle={t("mount.subtitle")}
      onSubmit={(e) => void submit(e)}
    >
      <FormGrid>
        <FormField label={t("mount.source")} span={2}>
          <ActionRow gap="md">
            <input
              required
              value={source}
              disabled={busy}
              onChange={(e) => setSource(e.target.value)}
              placeholder={t("mount.sourcePlaceholder")}
              style={{ flex: 1 }}
            />
            <Button
              tone="secondary"
              disabled={busy}
              onClick={() => void chooseSource()}
            >
              {t("mount.pick")}
            </Button>
          </ActionRow>
        </FormField>
        <FormField label={t("mount.target")}>
          <input
            required
            value={target}
            disabled={busy}
            onChange={(e) => setTarget(e.target.value)}
            placeholder={t("mount.targetPlaceholder")}
          />
        </FormField>
        <FormField label={t("mount.tag")}>
          <input
            value={tag}
            disabled={busy}
            onChange={(e) => setTag(e.target.value)}
            placeholder="app"
          />
        </FormField>
        <FormCheck>
          <input
            type="checkbox"
            checked={readOnly}
            disabled={busy}
            onChange={(e) => setReadOnly(e.target.checked)}
          />
          {t("mount.readOnly")}
        </FormCheck>
      </FormGrid>
      <FieldError message={error} />
      <FormActions
        busy={busy}
        submitLabel={busy ? t("mount.submitBusy") : t("mount.submit")}
        cancelLabel={t("common.cancel")}
        onCancel={onCancel}
      />
    </Card>
  );
}
