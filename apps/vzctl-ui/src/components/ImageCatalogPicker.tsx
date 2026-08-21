import { useEffect, useMemo, useState } from "react";
import { FormField } from "@/components/ui";
import { useT } from "@/lib/i18n";
import {
  catalogOsGroups,
  defaultCatalogSelection,
  parseCatalogAlias,
  resolveCatalogSelection,
  type ImageCatalogEntry,
} from "@/lib/images";

type ImageCatalogPickerProps = {
  value: string;
  onChange: (alias: string) => void;
  catalog: ImageCatalogEntry[];
  disabled?: boolean;
  osLabelKey?: "images.os" | "vmForm.os";
  versionLabelKey?: "images.version" | "vmForm.version";
};

export function ImageCatalogPicker({
  value,
  onChange,
  catalog,
  disabled = false,
  osLabelKey = "images.os",
  versionLabelKey = "images.version",
}: ImageCatalogPickerProps) {
  const t = useT();
  const groups = useMemo(() => catalogOsGroups(catalog), [catalog]);
  const parsed = useMemo(
    () => parseCatalogAlias(groups, value) ?? defaultCatalogSelection(groups, value),
    [groups, value],
  );
  const [distribution, setDistribution] = useState(parsed?.distribution ?? "");
  const [versionAlias, setVersionAlias] = useState(parsed?.versionAlias ?? value);

  useEffect(() => {
    const next = parseCatalogAlias(groups, value) ?? defaultCatalogSelection(groups, value);
    if (!next) return;
    setDistribution(next.distribution);
    setVersionAlias(next.versionAlias);
  }, [groups, value]);

  const activeGroup = groups.find((group) => group.distribution === distribution);
  const versions = activeGroup?.versions ?? [];

  useEffect(() => {
    if (versions.length === 0) return;
    if (versions.some((entry) => entry.alias === versionAlias)) return;
    const fallback = versions[0]?.alias;
    if (!fallback) return;
    setVersionAlias(fallback);
    onChange(resolveCatalogSelection(groups, distribution, fallback));
  }, [distribution, groups, onChange, versionAlias, versions]);

  if (groups.length === 0) {
    return (
      <FormField label={t(osLabelKey)}>
        <input value={value} disabled={disabled} readOnly />
      </FormField>
    );
  }

  return (
    <>
      <FormField label={t(osLabelKey)}>
        <select
          value={distribution}
          disabled={disabled}
          onChange={(event) => {
            const nextDistribution = event.target.value;
            const nextGroup = groups.find((group) => group.distribution === nextDistribution);
            const nextAlias = nextGroup?.versions[0]?.alias ?? value;
            setDistribution(nextDistribution);
            setVersionAlias(nextAlias);
            onChange(resolveCatalogSelection(groups, nextDistribution, nextAlias));
          }}
        >
          {groups.map((group) => (
            <option key={group.distribution} value={group.distribution}>
              {group.distribution}
            </option>
          ))}
        </select>
      </FormField>
      <FormField label={t(versionLabelKey)}>
        <select
          value={versionAlias}
          disabled={disabled || versions.length === 0}
          onChange={(event) => {
            const nextAlias = event.target.value;
            setVersionAlias(nextAlias);
            onChange(resolveCatalogSelection(groups, distribution, nextAlias));
          }}
        >
          {versions.map((entry) => (
            <option key={entry.alias} value={entry.alias}>
              {entry.label}
            </option>
          ))}
        </select>
      </FormField>
    </>
  );
}
