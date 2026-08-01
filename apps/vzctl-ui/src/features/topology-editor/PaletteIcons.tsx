import type { ReactNode } from "react";

type IconProps = {
  size?: number;
  className?: string;
  title?: string;
};

function SvgFrame({
  size = 28,
  className,
  title,
  children,
}: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-hidden={title ? undefined : true}
      role={title ? "img" : undefined}
    >
      {title ? <title>{title}</title> : null}
      {children}
    </svg>
  );
}

/** Cloud / network cloud shape */
export function IconNetwork({ size, className, title }: IconProps) {
  return (
    <SvgFrame size={size} className={className} title={title}>
      <rect x="2" y="2" width="28" height="28" rx="7" fill="#e7f3ef" />
      <path
        d="M10.5 21.5h11.2c2.1 0 3.8-1.6 3.8-3.6 0-1.8-1.3-3.3-3-3.6.1-.3.2-.7.2-1.1 0-2.4-2-4.4-4.5-4.4-1.7 0-3.2.9-4 2.2-.5-.3-1.1-.5-1.8-.5-1.9 0-3.4 1.5-3.4 3.3 0 .2 0 .4.1.6-1.7.3-3 1.8-3 3.5 0 1.9 1.6 3.6 3.4 3.6z"
        stroke="#0f6a5a"
        strokeWidth="1.5"
        fill="#fffaf0"
      />
      <circle cx="12" cy="18" r="1.2" fill="#0f6a5a" />
      <circle cx="16" cy="18" r="1.2" fill="#0f6a5a" />
      <circle cx="20" cy="18" r="1.2" fill="#0f6a5a" />
    </SvgFrame>
  );
}

/** Server / VM rack */
export function IconVm({ size, className, title }: IconProps) {
  return (
    <SvgFrame size={size} className={className} title={title}>
      <rect x="2" y="2" width="28" height="28" rx="7" fill="#fffaf0" />
      <rect
        x="8"
        y="7"
        width="16"
        height="18"
        rx="2.5"
        stroke="#1c2b27"
        strokeWidth="1.5"
        fill="#f3efe6"
      />
      <path d="M10.5 12h11" stroke="#1c2b27" strokeWidth="1.4" strokeLinecap="round" />
      <path d="M10.5 16.5h11" stroke="#1c2b27" strokeWidth="1.4" strokeLinecap="round" />
      <path d="M10.5 21h7" stroke="#1c2b27" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="20.5" cy="21" r="1.3" fill="#0f6a5a" />
    </SvgFrame>
  );
}

/** Router with antenna / routes */
export function IconRouter({ size, className, title }: IconProps) {
  return (
    <SvgFrame size={size} className={className} title={title}>
      <rect x="2" y="2" width="28" height="28" rx="7" fill="#faf0f0" />
      <rect
        x="7"
        y="14"
        width="18"
        height="10"
        rx="2"
        stroke="#9b2c2c"
        strokeWidth="1.5"
        fill="#fff"
      />
      <circle cx="11" cy="19" r="1.3" fill="#9b2c2c" />
      <circle cx="16" cy="19" r="1.3" fill="#9b2c2c" />
      <circle cx="21" cy="19" r="1.3" fill="#9b2c2c" />
      <path
        d="M12 14V9.5M16 14V8M20 14V9.5"
        stroke="#9b2c2c"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
      <circle cx="12" cy="8" r="1.4" fill="#9b2c2c" />
      <circle cx="16" cy="6.5" r="1.4" fill="#9b2c2c" />
      <circle cx="20" cy="8" r="1.4" fill="#9b2c2c" />
    </SvgFrame>
  );
}

/** Docker host / container role */
export function IconDocker({ size, className, title }: IconProps) {
  return (
    <SvgFrame size={size} className={className} title={title}>
      <rect x="2" y="2" width="28" height="28" rx="7" fill="#eef4fa" />
      <rect
        x="7"
        y="11"
        width="18"
        height="12"
        rx="2"
        stroke="#2a5a8a"
        strokeWidth="1.5"
        fill="#fffaf0"
      />
      <path
        d="M10 11V9.5c0-.8.7-1.5 1.5-1.5H14"
        stroke="#2a5a8a"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <path d="M11 15h2M15 15h2M19 15h2M11 18.5h10" stroke="#2a5a8a" strokeWidth="1.3" strokeLinecap="round" />
    </SvgFrame>
  );
}

export type PaletteKind = "network" | "vm" | "router" | "docker";

export function PaletteKindIcon({
  kind,
  size = 32,
}: {
  kind: PaletteKind;
  size?: number;
}) {
  switch (kind) {
    case "network":
      return <IconNetwork size={size} title="Netzwerk" />;
    case "router":
      return <IconRouter size={size} title="Router" />;
    case "docker":
      return <IconDocker size={size} title="Docker" />;
    default:
      return <IconVm size={size} title="Host" />;
  }
}
