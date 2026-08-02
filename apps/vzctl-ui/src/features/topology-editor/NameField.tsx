import { useEffect, useState, type KeyboardEvent } from "react";
import { FormField } from "@/components/ui";
import { useT } from "@/lib/i18n";

type Props = {
  label?: string;
  value: string;
  onCommit: (next: string) => void;
  disabled?: boolean;
};

/** Local draft; commits on blur or Enter. Escape resets. */
export function NameField({
  label,
  value,
  onCommit,
  disabled,
}: Props) {
  const t = useT();
  const fieldLabel = label ?? t("topo.field.name");
  const [draft, setDraft] = useState(value);

  useEffect(() => {
    setDraft(value);
  }, [value]);

  const commit = () => {
    const next = draft.trim();
    if (!next || next === value) {
      setDraft(value);
      return;
    }
    onCommit(next);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      (e.target as HTMLInputElement).blur();
    } else if (e.key === "Escape") {
      setDraft(value);
      (e.target as HTMLInputElement).blur();
    }
  };

  return (
    <FormField label={fieldLabel} variant="compact">
      <input
        type="text"
        value={draft}
        disabled={disabled}
        spellCheck={false}
        autoComplete="off"
        aria-label={fieldLabel}
        data-topology-name=""
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={onKeyDown}
      />
    </FormField>
  );
}
