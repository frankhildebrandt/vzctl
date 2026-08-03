import { useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { copyText } from "@/lib/clipboard";
import { useT } from "@/lib/i18n";
import { Button } from "./Button";
import { cx } from "./cx";

export type CopyButtonProps = Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "onClick"
> & {
  value: string;
  label?: ReactNode;
  copiedLabel?: ReactNode;
  tone?: "secondary" | "inline" | "overlay";
};

export function CopyButton({
  value,
  label,
  copiedLabel,
  tone = "secondary",
  className,
  disabled,
  ...props
}: CopyButtonProps) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const resolvedLabel = label ?? t("common.copy");
  const resolvedCopied = copiedLabel ?? t("common.copied");

  return (
    <Button
      tone="secondary"
      className={cx(
        tone === "inline" && "out-copy-inline",
        tone === "overlay" && "out-copy",
        className,
      )}
      disabled={disabled}
      onClick={() => {
        void copyText(value).then((ok) => {
          if (!ok) return;
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1500);
        });
      }}
      {...props}
    >
      {copied ? resolvedCopied : resolvedLabel}
    </Button>
  );
}

export type CodeBlockProps = {
  value: string;
  tone?: "default" | "error";
  copyable?: boolean;
  className?: string;
};

export function CodeBlock({
  value,
  tone = "default",
  copyable = false,
  className,
}: CodeBlockProps) {
  if (!copyable) {
    return (
      <pre className={cx("out", tone === "error" && "error", className)}>
        {value}
      </pre>
    );
  }

  return (
    <div className={cx("out-wrap", tone === "error" && "error", className)}>
      <CopyButton value={value} tone="overlay" />
      <pre className={cx("out", tone === "error" && "error")}>{value}</pre>
    </div>
  );
}

export type JsonBlockProps = {
  value: unknown;
  className?: string;
};

export function JsonBlock({ value, className }: JsonBlockProps) {
  let text: string;
  try {
    text =
      typeof value === "string" ? value : JSON.stringify(value, null, 2);
  } catch {
    text = String(value);
  }
  return <pre className={cx("inspect-json", className)}>{text}</pre>;
}
