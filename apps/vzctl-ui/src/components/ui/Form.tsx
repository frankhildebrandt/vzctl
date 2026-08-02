import type {
  InputHTMLAttributes,
  LabelHTMLAttributes,
  ReactNode,
  SelectHTMLAttributes,
  TextareaHTMLAttributes,
} from "react";
import { ActionRow } from "./ActionRow";
import { Button } from "./Button";
import { cx } from "./cx";

export type FormGridProps = {
  className?: string;
  children?: ReactNode;
};

export function FormGrid({ className, children }: FormGridProps) {
  return <div className={cx("form-grid", className)}>{children}</div>;
}

export type FieldErrorProps = {
  message?: string | null;
  className?: string;
};

export function FieldError({ message, className }: FieldErrorProps) {
  if (!message) return null;
  return <p className={cx("form-error", className)}>{message}</p>;
}

export type FormFieldProps = LabelHTMLAttributes<HTMLLabelElement> & {
  label: ReactNode;
  hint?: ReactNode;
  error?: string | null;
  span?: 1 | 2;
  /** Compact stacked field (topology-field). */
  variant?: "grid" | "compact";
  children: ReactNode;
};

export function FormField({
  label,
  hint,
  error,
  span,
  variant = "grid",
  className,
  children,
  ...props
}: FormFieldProps) {
  if (variant === "compact") {
    return (
      <label className={cx("topology-field", className)} {...props}>
        <span>{label}</span>
        {children}
        {hint != null ? <span className="muted">{hint}</span> : null}
        {error ? <span className="field-error">{error}</span> : null}
      </label>
    );
  }

  return (
    <label
      className={cx(span === 2 && "form-span-2", className)}
      {...props}
    >
      {label}
      {children}
      {hint != null ? <span className="muted">{hint}</span> : null}
      {error ? <span className="form-error">{error}</span> : null}
    </label>
  );
}

export type FormCheckProps = LabelHTMLAttributes<HTMLLabelElement> & {
  children: ReactNode;
  compact?: boolean;
};

export function FormCheck({
  children,
  compact,
  className,
  ...props
}: FormCheckProps) {
  return (
    <label
      className={cx(
        compact ? "topology-check" : "form-check",
        className,
      )}
      {...props}
    >
      {children}
    </label>
  );
}

export type FormActionsProps = {
  busy?: boolean;
  submitLabel: ReactNode;
  cancelLabel?: ReactNode;
  onCancel?: () => void;
  className?: string;
  /** Extra actions before cancel. */
  children?: ReactNode;
  submitType?: "submit" | "button";
  onSubmitClick?: () => void;
  submitDisabled?: boolean;
};

export function FormActions({
  busy,
  submitLabel,
  cancelLabel,
  onCancel,
  className,
  children,
  submitType = "submit",
  onSubmitClick,
  submitDisabled,
}: FormActionsProps) {
  return (
    <ActionRow gap="md" className={cx("settings-form-actions", className)}>
      <Button
        type={submitType}
        disabled={busy || submitDisabled}
        onClick={onSubmitClick}
      >
        {submitLabel}
      </Button>
      {children}
      {onCancel != null && cancelLabel != null ? (
        <Button tone="secondary" disabled={busy} onClick={onCancel}>
          {cancelLabel}
        </Button>
      ) : null}
    </ActionRow>
  );
}

export type TextInputProps = InputHTMLAttributes<HTMLInputElement>;
export type TextAreaProps = TextareaHTMLAttributes<HTMLTextAreaElement>;
export type SelectProps = SelectHTMLAttributes<HTMLSelectElement>;
