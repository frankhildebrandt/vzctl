import { Link } from "@tanstack/react-router";
import { useState, type MouseEvent, type ReactNode } from "react";
import { ChromeProvider } from "@/components/Chrome";
import { isDemoMode } from "@/lib/demo";
import { useT } from "@/lib/i18n";
import { invokeSidebarAction } from "@/lib/sidebarActions";
import { useSidebarNav, type SidebarNavItem } from "@/lib/sidebarNav";

export function AppShell({ children }: { children: ReactNode }) {
  const t = useT();
  const demo = isDemoMode();
  const nav = useSidebarNav();
  const [crumbsEl, setCrumbsEl] = useState<HTMLDivElement | null>(null);
  const [actionsEl, setActionsEl] = useState<HTMLDivElement | null>(null);
  const [noticeEl, setNoticeEl] = useState<HTMLDivElement | null>(null);

  return (
    <ChromeProvider crumbs={crumbsEl} actions={actionsEl} notice={noticeEl}>
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
                aria-label={t("shell.dashboardAria")}
              >
                <span className="sidebar-logo">{t("shell.backDashboard")}</span>
                <span className="sidebar-tag">{t("shell.brand")}</span>
              </Link>
            ) : (
              <div className="sidebar-brand" data-tauri-drag-region>
                <span className="sidebar-logo" data-tauri-drag-region>
                  {t("shell.brand")}
                </span>
                <span className="sidebar-tag" data-tauri-drag-region>
                  {t("shell.tagline")}
                </span>
              </div>
            )}
          </div>

          <nav className="sidebar-nav" aria-label={t("shell.navAria")}>
            <div key={nav.contextKey} className="sidebar-nav-panel">
              {nav.title ? (
                <div className="sidebar-context" title={nav.title}>
                  {nav.title}
                </div>
              ) : null}

              <div className="sidebar-notice" ref={setNoticeEl} />

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
                {t("shell.settings")}
              </Link>
            </div>
          ) : (
            <div className="sidebar-bottom" aria-hidden />
          )}
        </aside>
        <div className="main-column">
          <div
            className="content-chrome"
            data-tauri-drag-region
            onMouseDown={startWindowDrag}
          >
            {nav.back ? (
              <Link
                to={nav.back.to}
                params={nav.back.params}
                search={nav.back.search}
                className="chrome-back"
              >
                ← {nav.back.label}
              </Link>
            ) : null}
            <div className="chrome-crumbs" ref={setCrumbsEl} />
            <div className="chrome-spacer" data-tauri-drag-region aria-hidden />
            <div className="chrome-actions" ref={setActionsEl} />
          </div>
          <main className="content">{children}</main>
        </div>
        {demo ? (
          <span className="demo-watermark" role="status" aria-label={t("shell.demoAria")}>
            {t("shell.demo")}
          </span>
        ) : null}
      </div>
    </ChromeProvider>
  );
}

function NavLink({ item }: { item: SidebarNavItem }) {
  if (item.kind === "action") {
    return (
      <button
        type="button"
        className={
          item.tone === "danger"
            ? "sidebar-link sidebar-action tone-danger"
            : "sidebar-link sidebar-action"
        }
        disabled={item.disabled}
        onClick={() => invokeSidebarAction(item.id)}
      >
        {item.label}
      </button>
    );
  }

  const className = item.disabled
    ? "sidebar-link is-disabled"
    : item.active
      ? "sidebar-link active"
      : "sidebar-link";

  if (item.active !== undefined) {
    return (
      <Link
        to={item.to}
        params={item.params}
        search={item.search}
        className={className}
        aria-disabled={item.disabled || undefined}
        onClick={(event) => {
          if (item.disabled) event.preventDefault();
        }}
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
      className={item.disabled ? "sidebar-link is-disabled" : "sidebar-link"}
      activeProps={{ className: "sidebar-link active" }}
      aria-disabled={item.disabled || undefined}
      onClick={(event) => {
        if (item.disabled) event.preventDefault();
      }}
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
