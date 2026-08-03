import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "./cx";

export function SectionTitle({
  className,
  children,
  ...props
}: HTMLAttributes<HTMLHeadingElement>) {
  return (
    <h2 className={cx("section-title", className)} {...props}>
      {children}
    </h2>
  );
}

export function Muted({
  className,
  children,
  as: Tag = "p",
  ...props
}: HTMLAttributes<HTMLElement> & {
  as?: "p" | "span" | "div";
  children?: ReactNode;
}) {
  return (
    <Tag className={cx("muted", className)} {...props}>
      {children}
    </Tag>
  );
}

export function Mono({
  className,
  children,
  ...props
}: HTMLAttributes<HTMLElement>) {
  return (
    <code className={cx("mono", className)} {...props}>
      {children}
    </code>
  );
}

export function PathText({
  className,
  children,
  ...props
}: HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p className={cx("path", className)} {...props}>
      {children}
    </p>
  );
}
