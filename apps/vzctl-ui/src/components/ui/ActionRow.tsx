import type { HTMLAttributes } from "react";
import { cx } from "./cx";

export type ActionRowAlign = "start" | "end" | "between";
export type ActionRowGap = "sm" | "md" | "lg";

export type ActionRowProps = HTMLAttributes<HTMLDivElement> & {
  align?: ActionRowAlign;
  gap?: ActionRowGap;
};

const gapStyle: Record<ActionRowGap, string> = {
  sm: "0.35rem",
  md: "0.5rem",
  lg: "0.75rem",
};

const alignStyle: Record<ActionRowAlign, string> = {
  start: "flex-start",
  end: "flex-end",
  between: "space-between",
};

/** Horizontal action / toolbar row with semantic align/gap variants. */
export function ActionRow({
  align = "start",
  gap = "lg",
  className,
  style,
  children,
  ...props
}: ActionRowProps) {
  return (
    <div
      className={cx("row", className)}
      style={{
        gap: gapStyle[gap],
        justifyContent: alignStyle[align],
        ...style,
      }}
      {...props}
    >
      {children}
    </div>
  );
}

export type ToolbarProps = HTMLAttributes<HTMLDivElement>;

export function Toolbar({ className, children, ...props }: ToolbarProps) {
  return (
    <div className={cx("toolbar", className)} {...props}>
      {children}
    </div>
  );
}
