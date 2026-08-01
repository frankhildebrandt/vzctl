import { Link } from "@tanstack/react-router";
import type { MouseEvent, ReactNode } from "react";
import { isDemoMode } from "@/lib/demo";
import { useSidebarNav, type SidebarNavItem } from "@/lib/sidebarNav";

export function AppShell({ children }: { children: ReactNode }) {
  const demo = isDemoMode();
  const nav = useSidebarNav();

  return (
    <div className="app">
      <aside className="sidebar">
        <div
          className="titlebar"
          data-tauri-drag-region
          onMouseDown={startWindowDrag}
        >
          <div className="traffic-spacer" data-tauri-drag-region aria-hidden />
          {nav.showDashboard ? (
            <Link
              to="/"
              className="sidebar-brand sidebar-brand-link"
              aria-label="Zum Dashboard"
            >
              <span className="sidebar-logo">← Dashboard</span>
              <span className="sidebar-tag">vzctl</span>
            </Link>
          ) : (
            <div className="sidebar-brand" data-tauri-drag-region>
              <span className="sidebar-logo" data-tauri-drag-region>
                vzctl
              </span>
              <span className="sidebar-tag" data-tauri-drag-region>
                hypervisor
              </span>
            </div>
          )}
        </div>

        <nav className="sidebar-nav" aria-label="Hauptnavigation">
          <div key={nav.contextKey} className="sidebar-nav-panel">
            {nav.back ? (
              <Link
                to={nav.back.to}
                params={nav.back.params}
                search={nav.back.search}
                className="sidebar-link sidebar-back"
              >
                ← {nav.back.label}
              </Link>
            ) : null}

            {nav.title ? (
              <div className="sidebar-context" title={nav.title}>
                {nav.title}
              </div>
            ) : null}

            {nav.items.map((item) => (
              <NavLink key={item.id} item={item} />
            ))}
          </div>
        </nav>

        {nav.showSettingsBottom ? (
          <div className="sidebar-bottom">
            <Link
              to="/settings"
              className="sidebar-link"
              activeProps={{ className: "sidebar-link active" }}
            >
              Settings
            </Link>
          </div>
        ) : (
          <div className="sidebar-bottom" aria-hidden />
        )}
      </aside>
      <div className="main-column">
        <div
          className="content-drag"
          data-tauri-drag-region
          onMouseDown={startWindowDrag}
        />
        <main className="content">{children}</main>
      </div>
      {demo ? (
        <span className="demo-watermark" role="status" aria-label="Demo-Modus">
          Demo
        </span>
      ) : null}
    </div>
  );
}

function NavLink({ item }: { item: SidebarNavItem }) {
  if (item.active !== undefined) {
    return (
      <Link
        to={item.to}
        params={item.params}
        search={item.search}
        className={item.active ? "sidebar-link active" : "sidebar-link"}
      >
        {item.label}
      </Link>
    );
  }

  return (
    <Link
      to={item.to}
      params={item.params}
      search={item.search}
      activeOptions={{ exact: item.exact ?? false }}
      className="sidebar-link"
      activeProps={{ className: "sidebar-link active" }}
    >
      {item.label}
    </Link>
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
