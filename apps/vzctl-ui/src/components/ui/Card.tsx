import {
  type ComponentPropsWithoutRef,
  createElement,
  type ElementType,
  type ReactNode,
} from "react";
import { cx } from "./cx";
import { Muted } from "./Text";

export type CardTone = "default" | "error" | "summary";
export type CardPadding = "default" | "none";

type CardOwnProps = {
  title?: ReactNode;
  titleAs?: "h2" | "h3";
  subtitle?: ReactNode;
  tone?: CardTone;
  padding?: CardPadding;
  actions?: ReactNode;
  children?: ReactNode;
  className?: string;
  style?: React.CSSProperties;
};

export type CardProps<T extends ElementType = "div"> = CardOwnProps &
  Omit<ComponentPropsWithoutRef<T>, keyof CardOwnProps | "as"> & {
    as?: T;
  };

export function Card<T extends ElementType = "div">({
  as,
  title,
  titleAs = "h2",
  subtitle,
  tone = "default",
  padding = "default",
  actions,
  className,
  children,
  style,
  ...props
}: CardProps<T>) {
  const Tag = (as ?? "div") as ElementType;
  const TitleTag = titleAs;
  return createElement(
    Tag,
    {
      className: cx(
        "card",
        tone === "error" && "error-card",
        tone === "summary" && "summary-card",
        className,
      ),
      style:
        padding === "none"
          ? { padding: 0, overflow: "auto", ...style }
          : style,
      ...props,
    },
    <>
      {title != null || actions != null ? (
        actions != null ? (
          <div className="summary-row">
            <div>
              {title != null ? <TitleTag>{title}</TitleTag> : null}
              {subtitle != null ? <Muted>{subtitle}</Muted> : null}
            </div>
            {actions}
          </div>
        ) : (
          <>
            {title != null ? <TitleTag>{title}</TitleTag> : null}
            {subtitle != null ? <Muted>{subtitle}</Muted> : null}
          </>
        )
      ) : subtitle != null ? (
        <Muted>{subtitle}</Muted>
      ) : null}
      {children}
    </>,
  );
}
