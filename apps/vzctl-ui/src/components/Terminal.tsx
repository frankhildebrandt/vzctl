import { lazy, Suspense, type ComponentProps } from "react";
import { useT } from "@/lib/i18n";

const TerminalView = lazy(() => import("./TerminalView").then((module) => ({ default: module.Terminal })));

/** Load xterm only when a terminal session is opened. */
export function Terminal(props: ComponentProps<typeof TerminalView>) {
  const t = useT();
  return <Suspense fallback={<p>{t("common.loading")}</p>}><TerminalView {...props} /></Suspense>;
}
