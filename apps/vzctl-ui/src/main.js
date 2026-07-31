import { invoke } from "@tauri-apps/api/core";

const app = document.getElementById("app");

const state = {
  path: null,
  output: "",
  busy: false,
};

function render() {
  app.innerHTML = `
    <main class="shell">
      <header>
        <h1>vzctl</h1>
        <p class="muted">CLI-backed environment control — no second reconciler</p>
      </header>
      <section class="row">
        <button id="open">Open Environment…</button>
        <span class="path">${state.path ?? "no environment open"}</span>
      </section>
      <section class="actions">
        <button data-cmd="diff" ${disabled()}>Diff</button>
        <button data-cmd="up" ${disabled()}>Up</button>
        <button data-cmd="apply" ${disabled()}>Apply</button>
        <button data-cmd="down" ${disabled()}>Down</button>
        <button data-cmd="status" ${disabled()}>DNS/OIDC/CA</button>
      </section>
      <pre class="out">${escapeHtml(state.output || "Ready.")}</pre>
    </main>
  `;

  document.getElementById("open").onclick = async () => {
    try {
      const path = await invoke("open_environment");
      if (path) {
        state.path = path;
        state.output = `Opened ${path}`;
        render();
      }
    } catch (error) {
      state.output = String(error);
      render();
    }
  };

  for (const button of document.querySelectorAll("[data-cmd]")) {
    button.onclick = async () => {
      if (!state.path || state.busy) return;
      state.busy = true;
      render();
      try {
        state.output = await invoke("run_vzctl", {
          path: state.path,
          command: button.dataset.cmd,
        });
      } catch (error) {
        state.output = String(error);
      } finally {
        state.busy = false;
        render();
      }
    };
  }
}

function disabled() {
  return !state.path || state.busy ? "disabled" : "";
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

const style = document.createElement("style");
style.textContent = `
  :root {
    color-scheme: light;
    --bg: #f3efe6;
    --ink: #1c1915;
    --accent: #0f6a5a;
    --panel: #fffaf0;
    font-family: "IBM Plex Sans", "Segoe UI", sans-serif;
  }
  body { margin: 0; background: radial-gradient(circle at top left, #dff5ef, var(--bg)); color: var(--ink); }
  .shell { max-width: 920px; margin: 0 auto; padding: 2rem; }
  h1 { font-size: 2.4rem; margin: 0 0 .25rem; letter-spacing: -.03em; }
  .muted { color: #5b564c; margin: 0 0 1.5rem; }
  .row, .actions { display: flex; gap: .75rem; align-items: center; flex-wrap: wrap; margin-bottom: 1rem; }
  button {
    background: var(--accent); color: white; border: 0; border-radius: 8px;
    padding: .65rem 1rem; font: inherit; cursor: pointer;
  }
  button:disabled { opacity: .45; cursor: not-allowed; }
  .path { font-family: "IBM Plex Mono", ui-monospace, monospace; font-size: .9rem; }
  .out {
    background: var(--panel); border: 1px solid #ddd2c0; border-radius: 12px;
    padding: 1rem; min-height: 280px; overflow: auto; white-space: pre-wrap;
    font-family: "IBM Plex Mono", ui-monospace, monospace; font-size: .85rem;
  }
`;
document.head.appendChild(style);
render();
