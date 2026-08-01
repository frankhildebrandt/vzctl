import { Link } from "@tanstack/react-router";
import type { MouseEvent, ReactNode } from "react";

const NAV: Array<{
  to: "/" | "/vms" | "/projects" | "/networks" | "/images" | "/doctor";
  label: string;
  exact?: boolean;
}> = [
  { to: "/", label: "Dashboard", exact: true },
  { to: "/vms", label: "VMs" },
  { to: "/projects", label: "Stacks" },
  { to: "/networks", label: "Networks" },
  { to: "/images", label: "Images" },
  { to: "/doctor", label: "Doctor" },
];

export function AppShell({ children }: { children: ReactNode }) {
  return (
    <div className="app">
      <aside className="sidebar">
        <div
          className="titlebar"
          data-tauri-drag-region
          onMouseDown={startWindowDrag}
        >
          <div className="traffic-spacer" data-tauri-drag-region aria-hidden />
          <div className="sidebar-brand" data-tauri-drag-region>
            <span className="sidebar-logo" data-tauri-drag-region>
              vzctl
            </span>
            <span className="sidebar-tag" data-tauri-drag-region>
              hypervisor
            </span>
          </div>
        </div>
        <nav className="sidebar-nav">
          {NAV.map((item) => (
            <Link
              key={item.to}
              to={item.to}
              activeOptions={{ exact: item.exact ?? false }}
              className="sidebar-link"
              activeProps={{ className: "sidebar-link active" }}
            >
              {item.label}
            </Link>
          ))}
        </nav>
        <p className="sidebar-foot muted">CLI-backed · kein zweiter Reconciler</p>
      </aside>
      <div className="main-column">
        <div
          className="content-drag"
          data-tauri-drag-region
          onMouseDown={startWindowDrag}
        />
        <main className="content">{children}</main>
      </div>
    </div>
  );
}

function startWindowDrag(event: MouseEvent<HTMLElement>) {
  if (event.button !== 0) return;
  const target = event.target as HTMLElement | null;
  if (target?.closest("a, button, input, textarea, select, label")) return;
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow().startDragging())
    .catch(() => {
      // Browser preview without Tauri — ignore.
    });
}
