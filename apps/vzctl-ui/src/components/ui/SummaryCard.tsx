import type { HTMLAttributes, ReactNode } from "react";
import { Badge, type BadgeTone } from "./Badge";
import { Card } from "./Card";
import { cx } from "./cx";
import { Muted } from "./Text";

export type SummaryCardProps = HTMLAttributes<HTMLDivElement> & {
  badge?: ReactNode;
  badgeTone?: BadgeTone;
  badgeLabel?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
};

export function SummaryCard({
  badge,
  badgeTone,
  badgeLabel,
  meta,
  actions,
  className,
  children,
  ...props
}: SummaryCardProps) {
  return (
    <Card tone="summary" className={className} {...props}>
      <div className="summary-row">
        {badge != null
          ? badge
          : badgeLabel != null
            ? (
              <Badge tone={badgeTone ?? "neutral"}>{badgeLabel}</Badge>
            )
            : null}
        {meta != null ? (
          typeof meta === "string" ? <Muted as="span">{meta}</Muted> : meta
        ) : null}
        {actions}
      </div>
      {children}
    </Card>
  );
}

export type SummaryRowProps = HTMLAttributes<HTMLDivElement> & {
  children?: ReactNode;
};

export function SummaryRow({ className, children, ...props }: SummaryRowProps) {
  return (
    <div className={cx("summary-row", className)} {...props}>
      {children}
    </div>
  );
}
