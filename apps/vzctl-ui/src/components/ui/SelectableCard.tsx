import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cx } from "./cx";

export type SelectableCardProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  selected?: boolean;
  label: ReactNode;
  description?: ReactNode;
  preview?: ReactNode;
  /** Visual variant: theme grid card vs compact provider preset. */
  appearance?: "theme" | "locale" | "preset";
  previewKey?: string;
};

export function SelectableCard({
  selected,
  label,
  description,
  preview,
  appearance = "theme",
  previewKey,
  className,
  type = "button",
  role = "radio",
  ...props
}: SelectableCardProps) {
  const base =
    appearance === "preset"
      ? "provider-preset"
      : appearance === "locale"
        ? "theme-card locale-card"
        : "theme-card";

  return (
    <button
      type={type}
      role={role}
      aria-checked={selected}
      data-preview={previewKey}
      className={cx(base, selected && "selected", className)}
      {...props}
    >
      {preview}
      <span className="theme-card-meta">
        <span className="theme-card-label">{label}</span>
        {description != null ? (
          <span className="theme-card-desc">{description}</span>
        ) : null}
      </span>
    </button>
  );
}
