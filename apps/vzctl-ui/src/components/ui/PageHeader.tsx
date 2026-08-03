import type { HTMLAttributes, ReactNode } from "react";
import { ActionRow } from "./ActionRow";
import { cx } from "./cx";
import { Muted, SectionTitle } from "./Text";

export type PageHeaderProps = HTMLAttributes<HTMLElement> & {
  title: ReactNode;
  subtitle?: ReactNode;
  breadcrumbs?: ReactNode;
  actions?: ReactNode;
  /** Use detail-heading layout (compact) vs row-between layout. */
  layout?: "row" | "detail";
};

export function PageHeader({
  title,
  subtitle,
  breadcrumbs,
  actions,
  layout = "row",
  className,
  style,
  ...props
}: PageHeaderProps) {
  if (layout === "detail") {
    return (
      <header
        className={cx("detail-heading", className)}
        style={{ marginBottom: "1rem", ...style }}
        {...props}
      >
        {breadcrumbs}
        <SectionTitle>{title}</SectionTitle>
        {subtitle != null ? <Muted>{subtitle}</Muted> : null}
        {actions}
      </header>
    );
  }

  return (
    <ActionRow
      align="between"
      className={className}
      style={style}
      {...props}
    >
      <div>
        {breadcrumbs}
        <SectionTitle>{title}</SectionTitle>
        {subtitle != null ? <Muted>{subtitle}</Muted> : null}
      </div>
      {actions}
    </ActionRow>
  );
}
