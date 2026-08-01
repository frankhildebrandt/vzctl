import { Fragment, type ReactNode } from "react";
import { useT } from "@/lib/i18n";

export type BreadcrumbItem = {
  label: string;
  /** Link-Inhalt; ohne `node` = aktuelle Seite (nicht klickbar). */
  node?: ReactNode;
};

export function Breadcrumbs({ items }: { items: BreadcrumbItem[] }) {
  const t = useT();
  if (items.length === 0) return null;

  return (
    <nav className="crumb" aria-label={t("crumb.aria")}>
      {items.map((item, index) => (
        <Fragment key={`${item.label}-${index}`}>
          {index > 0 ? (
            <span className="crumb-sep" aria-hidden="true">
              /
            </span>
          ) : null}
          {item.node ?? (
            <span className="crumb-current" aria-current="page">
              {item.label}
            </span>
          )}
        </Fragment>
      ))}
    </nav>
  );
}
