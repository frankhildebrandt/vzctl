import type { HTMLAttributes } from "react";
import { cx } from "./cx";

export type BadgeTone = "neutral" | "ok" | "warn" | "danger";

export type BadgeProps = HTMLAttributes<HTMLSpanElement> & {
  tone?: BadgeTone;
};

const toneClass: Record<BadgeTone, string | undefined> = {
  neutral: undefined,
  ok: "ok",
  warn: "warn",
  danger: "danger",
};

export function Badge({
  tone = "neutral",
  className,
  children,
  ...props
}: BadgeProps) {
  return (
    <span className={cx("badge", toneClass[tone], className)} {...props}>
      {children}
    </span>
  );
}

export type StatusPillProps = HTMLAttributes<HTMLSpanElement> & {
  /** VM / container runtime state token (running, stopped, …). */
  state: string;
};

/** Runtime state pill using `.vm-state.state-*` tokens. */
export function StatusPill({
  state,
  className,
  children,
  ...props
}: StatusPillProps) {
  return (
    <span
      className={cx("vm-state", `state-${state}`, className)}
      {...props}
    >
      {children ?? state}
    </span>
  );
}

export type StackPhasePillProps = HTMLAttributes<HTMLSpanElement> & {
  phase: string;
  loading?: boolean;
};

export function StackPhasePill({
  phase,
  loading,
  className,
  children,
  ...props
}: StackPhasePillProps) {
  return (
    <span
      className={cx("stack-pill", `phase-${phase}`, className)}
      {...props}
    >
      {loading ? "…" : children}
    </span>
  );
}
