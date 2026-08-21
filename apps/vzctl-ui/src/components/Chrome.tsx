import {
  createContext,
  useContext,
  useMemo,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

type ChromeTargets = {
  crumbs: HTMLElement | null;
  actions: HTMLElement | null;
  notice: HTMLElement | null;
};

const ChromeContext = createContext<ChromeTargets>({
  crumbs: null,
  actions: null,
  notice: null,
});

/** Provides portal targets for the main-column top chrome. */
export function ChromeProvider({
  crumbs,
  actions,
  notice,
  children,
}: {
  crumbs: HTMLElement | null;
  actions: HTMLElement | null;
  notice: HTMLElement | null;
  children: ReactNode;
}) {
  const value = useMemo(
    () => ({ crumbs, actions, notice }),
    [crumbs, actions, notice],
  );
  return (
    <ChromeContext.Provider value={value}>{children}</ChromeContext.Provider>
  );
}

/** Render breadcrumbs into the top chrome, beside Zurück. */
export function ChromeCrumbs({ children }: { children: ReactNode }) {
  const { crumbs } = useContext(ChromeContext);
  if (!crumbs) return null;
  return createPortal(children, crumbs);
}

/** Render page actions (e.g. start/stop/restart) into the top-right chrome. */
export function ChromeActions({ children }: { children: ReactNode }) {
  const { actions } = useContext(ChromeContext);
  if (!actions) return null;
  return createPortal(children, actions);
}

/** Render a notice into the contextual sidebar (below nav items). */
export function ChromeSidebarNotice({ children }: { children: ReactNode }) {
  const { notice } = useContext(ChromeContext);
  if (!notice) return null;
  return createPortal(children, notice);
}
