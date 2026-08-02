import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cx } from "./cx";

export type ButtonTone = "primary" | "secondary" | "danger" | "quiet";

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  tone?: ButtonTone;
};

const toneClass: Record<ButtonTone, string | undefined> = {
  primary: undefined,
  secondary: "secondary",
  danger: "danger",
  quiet: "secondary",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  function Button(
    { tone = "primary", type = "button", className, ...props },
    ref,
  ) {
    return (
      <button
        ref={ref}
        type={type}
        className={cx(toneClass[tone], className)}
        {...props}
      />
    );
  },
);
