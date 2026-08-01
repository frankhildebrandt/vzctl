import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { getT } from "@/lib/i18n";

type Mode = "attach" | "exec";

type TerminalDataEvent = {
  sessionId: string;
  data: number[];
};

type TerminalExitEvent = {
  sessionId: string;
  code?: number | null;
  message?: string;
};

export function Terminal({
  mode,
  vmId,
  cmd = ["/bin/bash"],
}: {
  mode: Mode;
  vmId: string;
  cmd?: string[];
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const cmdKey = cmd.join("\0");

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const command = cmdKey.split("\0").filter(Boolean);

    const term = new XTerm({
      cursorBlink: true,
      convertEol: true,
      fontFamily: '"IBM Plex Mono", "SF Mono", Menlo, monospace',
      fontSize: 13,
      theme: {
        background: "#14201c",
        foreground: "#d7e8e2",
        cursor: "#7dcaa8",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();

    let sessionId: string | null = null;
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    let resizeObserver: ResizeObserver | null = null;

    async function openSession() {
      const { invoke } = await import("@tauri-apps/api/core");
      const cols = term.cols;
      const rows = term.rows;

      if (mode === "attach") {
        sessionId = await invoke<string>("terminal_open_attach", { vmId });
      } else {
        sessionId = await invoke<string>("terminal_open_exec", {
          vmId,
          cmd: command.length ? command : ["/bin/bash"],
          cols,
          rows,
        });
      }
      if (disposed) {
        if (sessionId) {
          await invoke("terminal_close", { sessionId });
        }
        return;
      }

      const currentSession = sessionId;
      term.onData((data) => {
        if (!currentSession) return;
        const bytes = Array.from(new TextEncoder().encode(data));
        void invoke("terminal_write", { sessionId: currentSession, data: bytes });
      });

      unlisteners.push(
        await listen<TerminalDataEvent>("terminal-data", (event) => {
          if (event.payload.sessionId !== currentSession) return;
          term.write(Uint8Array.from(event.payload.data));
        }),
      );
      unlisteners.push(
        await listen<TerminalExitEvent>("terminal-exit", (event) => {
          if (event.payload.sessionId !== currentSession) return;
          const t = getT();
          const code = event.payload.code;
          const msg =
            event.payload.message ??
            (code == null
              ? t("terminal.sessionClosed")
              : t("terminal.exit", { code: String(code) }));
          term.writeln(`\r\n\x1b[90m[${msg}]\x1b[0m`);
        }),
      );

      resizeObserver = new ResizeObserver(() => {
        fit.fit();
        if (mode === "exec" && currentSession) {
          void invoke("terminal_resize", {
            sessionId: currentSession,
            cols: term.cols,
            rows: term.rows,
          });
        }
      });
      if (host) resizeObserver.observe(host);
    }

    void openSession().catch((err) => {
      term.writeln(`\x1b[31m${String(err)}\x1b[0m`);
    });

    return () => {
      disposed = true;
      resizeObserver?.disconnect();
      for (const unlisten of unlisteners) unlisten();
      if (sessionId) {
        void import("@tauri-apps/api/core").then(({ invoke }) =>
          invoke("terminal_close", { sessionId }),
        );
      }
      term.dispose();
    };
  }, [mode, vmId, cmdKey]);

  return <div className="terminal-host" ref={hostRef} />;
}
