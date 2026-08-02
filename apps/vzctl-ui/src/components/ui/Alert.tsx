import type { HTMLAttributes, ReactNode } from "react";
import { Card } from "./Card";
import { cx } from "./cx";
import { Muted } from "./Text";

export type AlertTone = "danger" | "warn" | "info";

export type AlertProps = HTMLAttributes<HTMLDivElement> & {
  tone?: AlertTone;
  title?: ReactNode;
  children?: ReactNode;
};

/** Inline notice / error card. Defaults to danger (error-card). */
export function Alert({
  tone = "danger",
  title,
  className,
  children,
  role = "alert",
  ...props
}: AlertProps) {
  return (
    <Card
      tone={tone === "danger" ? "error" : "default"}
      title={title}
      titleAs="h3"
      className={cx(tone === "warn" && "topology-banner warn", className)}
      role={role}
      {...props}
    >
      {typeof children === "string" || typeof children === "number" ? (
        <p>{children}</p>
      ) : (
        children
      )}
    </Card>
  );
}

export type EmptyStateProps = {
  title?: ReactNode;
  message: ReactNode;
  action?: ReactNode;
  /** Wrap in a card (default true). */
  card?: boolean;
  className?: string;
};

export function EmptyState({
  title,
  message,
  action,
  card = true,
  className,
}: EmptyStateProps) {
  const body = (
    <>
      {title != null ? <h2>{title}</h2> : null}
      {typeof message === "string" ? <Muted>{message}</Muted> : message}
      {action}
    </>
  );
  if (!card) {
    return <div className={className}>{body}</div>;
  }
  return <Card className={className}>{body}</Card>;
}

export type LoadingStateProps = {
  message: ReactNode;
  card?: boolean;
  className?: string;
};

export function LoadingState({
  message,
  card = false,
  className,
}: LoadingStateProps) {
  if (card) {
    return (
      <Card className={cx("result-empty", className)}>
        <Muted>{message}</Muted>
      </Card>
    );
  }
  return <Muted className={className}>{message}</Muted>;
}
