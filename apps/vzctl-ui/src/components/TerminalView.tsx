import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { getT, useT } from "@/lib/i18n";

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

/** xterm view that can reconnect without dropping scrollback. */
export function Terminal({
  mode,
  vmId,
  cmd = ["/bin/bash"],
  active = true,
}: {
  mode: Mode;
  vmId: string;
  cmd?: string[];
  active?: boolean;
}) {
  const t = useT();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const [closed, setClosed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [epoch, setEpoch] = useState(0);
  const [ready, setReady] = useState(false);
  const cmdKey = cmd.join("\0");

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
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
    termRef.current = term;
    fitRef.current = fit;
    setReady(true);
    term.onData((data) => {
      const sessionId = sessionRef.current;
      if (!sessionId) return;
      const bytes = Array.from(new TextEncoder().encode(data));
      void import("@tauri-apps/api/core").then(({ invoke }) =>
        invoke("terminal_write", { sessionId, data: bytes }),
      );
    });
    const resizeObserver = new ResizeObserver(() => {
      fit.fit();
      const sessionId = sessionRef.current;
      if (mode === "exec" && sessionId) {
        void import("@tauri-apps/api/core").then(({ invoke }) =>
          invoke("terminal_resize", {
            sessionId,
            cols: term.cols,
            rows: term.rows,
          }),
        );
      }
    });
    resizeObserver.observe(host);
    return () => {
      resizeObserver.disconnect();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      setReady(false);
    };
  }, [mode]);

  useEffect(() => {
    if (!active) return;
    fitRef.current?.fit();
  }, [active]);

  useEffect(() => {
    const term = termRef.current;
    if (!term || !ready) return;
    const xterm = term;
    const command = cmdKey.split("\0").filter(Boolean);
    let sessionId: string | null = null;
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];

    async function openSession() {
      const { invoke } = await import("@tauri-apps/api/core");
      const cols = xterm.cols;
      const rows = xterm.rows;
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
        if (sessionId) await invoke("terminal_close", { sessionId });
        return;
      }
      sessionRef.current = sessionId;
      setClosed(false);
      setError(null);
      const currentSession = sessionId;
      unlisteners.push(
        await listen<TerminalDataEvent>("terminal-data", (event) => {
          if (event.payload.sessionId !== currentSession) return;
          xterm.write(Uint8Array.from(event.payload.data));
        }),
      );
      unlisteners.push(
        await listen<TerminalExitEvent>("terminal-exit", (event) => {
          if (event.payload.sessionId !== currentSession) return;
          sessionRef.current = null;
          setClosed(true);
          const translate = getT();
          const code = event.payload.code;
          const msg =
            event.payload.message ??
            (code == null
              ? translate("terminal.sessionClosed")
              : translate("terminal.exit", { code: String(code) }));
          xterm.writeln(`\r\n\x1b[90m[${msg}]\x1b[0m`);
        }),
      );
    }

    void openSession().catch((err) => {
      if (disposed) return;
      setClosed(true);
      setError(String(err));
      xterm.writeln(`\x1b[31m${String(err)}\x1b[0m`);
    });

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
      const closing = sessionId ?? sessionRef.current;
      sessionRef.current = null;
      if (closing) {
        void import("@tauri-apps/api/core").then(({ invoke }) =>
          invoke("terminal_close", { sessionId: closing }),
        );
      }
    };
  }, [mode, vmId, cmdKey, epoch, ready]);

  return (
    <div className="terminal-frame">
      <div className="terminal-host" ref={hostRef} />
      {closed ? (
        <div className="terminal-reconnect">
          <p>{error ?? t("terminal.sessionClosed")}</p>
          <button
            type="button"
            className="secondary"
            onClick={() => {
              termRef.current?.writeln(
                `\x1b[90m[${t("terminal.reconnecting")}]\x1b[0m`,
              );
              setClosed(false);
              setError(null);
              setEpoch((value) => value + 1);
            }}
          >
            {t("terminal.reconnect")}
          </button>
        </div>
      ) : null}
    </div>
  );
}
