import type { ReactNode, SVGProps } from "react";
import { cx } from "@/components/ui/cx";

type IconProps = SVGProps<SVGSVGElement>;

function Icon(props: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      {...props}
    />
  );
}

export function IconPlay(props: IconProps) {
  return (
    <Icon {...props}>
      <polygon points="7 4 20 12 7 20 7 4" fill="currentColor" stroke="none" />
    </Icon>
  );
}

export function IconStop(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor" stroke="none" />
    </Icon>
  );
}

export function IconDiff(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 4v16" />
      <path d="M16 4v16" />
      <path d="M5 9h6" />
      <path d="M13 15h6" />
    </Icon>
  );
}

export function IconApply(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M20 7 10 17l-5-5" />
    </Icon>
  );
}

export function IconStatus(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 3 4.5 6v5c0 4.5 3.2 8.2 7.5 9.5 4.3-1.3 7.5-5 7.5-9.5V6L12 3z" />
      <path d="M9.5 12h5" />
      <path d="M12 9.5v5" />
    </Icon>
  );
}

export function IconTrash(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4 7h16" />
      <path d="M9 7V5h6v2" />
      <path d="M7 7l1 13h8l1-13" />
    </Icon>
  );
}

export function IconPurge(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4 7h16" />
      <path d="M9 7V5h6v2" />
      <path d="M7 7l1 13h8l1-13" />
      <path d="M3 3l18 18" />
    </Icon>
  );
}

export function IconSave(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M5 4h11l3 3v13H5z" />
      <path d="M8 4v5h8V4" />
      <path d="M8 20v-6h8v6" />
    </Icon>
  );
}

export function IconUndo(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M9 14 4 9l5-5" />
      <path d="M4 9h9a6 6 0 0 1 0 12h-2" />
    </Icon>
  );
}

export function IconRedo(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M15 14l5-5-5-5" />
      <path d="M20 9H11a6 6 0 0 0 0 12h2" />
    </Icon>
  );
}

export function IconFit(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4 9V4h5" />
      <path d="M20 9V4h-5" />
      <path d="M4 15v5h5" />
      <path d="M20 15v5h-5" />
    </Icon>
  );
}

export function IconSettings(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2v2" />
      <path d="M12 20v2" />
      <path d="M4.9 4.9l1.4 1.4" />
      <path d="M17.7 17.7l1.4 1.4" />
      <path d="M2 12h2" />
      <path d="M20 12h2" />
      <path d="M4.9 19.1l1.4-1.4" />
      <path d="M17.7 6.3l1.4-1.4" />
    </Icon>
  );
}

export function IconLayout(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="4" y="4" width="7" height="7" rx="1" />
      <rect x="13" y="4" width="7" height="7" rx="1" />
      <rect x="4" y="13" width="7" height="7" rx="1" />
      <rect x="13" y="13" width="7" height="7" rx="1" />
    </Icon>
  );
}

export function IconButton({
  label,
  disabled,
  onClick,
  tone = "default",
  showLabel = false,
  children,
}: {
  label: string;
  disabled?: boolean;
  onClick?: () => void;
  tone?: "default" | "primary" | "danger" | "quiet";
  /** Show the label next to the icon (stack actions). */
  showLabel?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={cx("icon-btn", `tone-${tone}`, showLabel && "with-label")}
      disabled={disabled}
      onClick={onClick}
      title={label}
      aria-label={label}
    >
      {children}
      {showLabel ? <span className="icon-btn-label">{label}</span> : null}
    </button>
  );
}
