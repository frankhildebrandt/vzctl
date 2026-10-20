import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ResultPanel } from "@/components/ResultPanel";
import { StackHealthSummary } from "@/components/StackHealthSummary";
import { parseStackHealth } from "@/lib/stackHealth";
import { translate } from "@/lib/i18n";

const bundle = (health: unknown) => ({ sections: {
  stack: { ok: true, data: { phase: "running", label: "Up (Running)", vms: { running: 3, desired: 3 } } },
  health,
  oidc: { ok: true, data: { enabled: false, status: "disabled" } },
} });

describe("stack health contract", () => {
  it.each([undefined, null, { ok: false, data: null }, { ok: true, data: {} }, { ok: true, data: { summary: { health: "new-value" } } }])("keeps unavailable or incomplete health unknown", (health) => {
    expect(parseStackHealth(bundle(health)).state).toBe("unknown");
  });
  it("keeps diagnostics degraded while the process inventory is running", () => {
    const health = parseStackHealth(bundle({ ok: true, data: { summary: { health: "degraded" }, warnings: [{ message: "DNS timeout", vm_id: "neti/web", hint: "Check DNS" }] } }));
    expect(health).toEqual({ state: "degraded", warnings: [{ message: "DNS timeout", vmId: "neti/web", hint: "Check DNS" }] });
  });
  it("renders degraded/unknown health and disabled OIDC accurately", () => {
    const degraded = bundle({ ok: true, data: { summary: { health: "degraded" }, warnings: [{ message: "DNS timeout" }] } });
    const html = renderToStaticMarkup(<ResultPanel result={{ kind: "status", raw: JSON.stringify(degraded) }} />);
    expect(html).toContain("Up (Running)");
    expect(html).toContain("DNS timeout");
    expect(html).toContain("Health: degraded");
    expect(html).toContain("disabled");
    expect(html).not.toContain('>stopped<');
    const unknown = renderToStaticMarkup(<StackHealthSummary health={parseStackHealth(bundle(undefined))} />);
    expect(unknown).toContain("Health: unknown");
    expect(unknown).not.toContain('class="badge ok"');
  });
  it("has distinct translated health and disabled labels in both locales", () => {
    expect(translate("deDE", "stack.health.degraded")).toBe("Gesundheit: eingeschränkt");
    expect(translate("enUS", "stack.health.degraded")).toBe("Health: degraded");
    expect(translate("deDE", "stack.health.unknown")).toBe("Gesundheit: unbekannt");
    expect(translate("enUS", "result.status.disabled")).toBe("disabled");
    expect(translate("deDE", "result.status.disabled")).toBe("deaktiviert");
  });
  it.each(["degraded", "critical"])("accepts a valid %s report with a nonzero CLI exit", (state) => {
    expect(parseStackHealth(bundle({ ok: false, data: { summary: { health: state }, warnings: [{ message: "DNS timeout" }] } })).state).toBe(state);
  });
  it("does not accept stale successful data after failed diagnostics", () => {
    expect(parseStackHealth(bundle({ ok: false, data: { summary: { health: "ok" } }, stderr: "timeout" })))
      .toEqual({ state: "unknown", warnings: [], error: "timeout" });
  });
});


describe("stopped VM health", () => {
  it("renders a confirmed stopped stack neutrally, including the status row", () => {
    const stopped = bundle({ ok: true, data: { summary: { health: "ok", checks: "skipped", stopped_vms: 3 }, warnings: [] } });
    const health = parseStackHealth(stopped);
    expect(health).toEqual({ state: "stopped", warnings: [], stoppedVms: 3 });
    const html = renderToStaticMarkup(<StackHealthSummary health={health} />);
    expect(html).toContain('class="badge">Stopped · not checked');
    expect(html).toContain("3 stopped VM(s): dependent checks skipped.");
    expect(html).not.toContain('badge ok');
    const result = renderToStaticMarkup(<ResultPanel result={{ kind: "status", raw: JSON.stringify(stopped) }} />);
    expect(result).toContain('class="badge">Stopped · not checked');
  });
  it.each(["degraded", "critical"])("retains independent %s errors even when VM checks were skipped", (state) => {
    const health = parseStackHealth(bundle({ ok: false, data: { summary: { health: state, checks: "skipped", stopped_vms: 1 }, warnings: [{ message: "Active service failed" }] } }));
    expect(health.state).toBe(state);
    const html = renderToStaticMarkup(<StackHealthSummary health={health} />);
    expect(html).toContain("Active service failed");
    expect(html).toContain("dependent checks skipped");
    expect(html).not.toContain("Stopped · not checked");
  });
  it("keeps active VM health when only some VMs are stopped", () => {
    expect(parseStackHealth(bundle({ ok: true, data: { summary: { health: "ok", checks: "evaluated", stopped_vms: 1 } } })))
      .toEqual({ state: "ok", warnings: [], stoppedVms: 1 });
  });
  it("does not infer a stop from stale data or an unavailable inventory", () => {
    expect(parseStackHealth(bundle({ ok: false, data: { summary: { health: "ok", checks: "skipped" } }, stderr: "VM inventory unavailable" })).state).toBe("unknown");
    expect(translate("deDE", "stack.health.stopped")).toBe("Gestoppt · nicht geprüft");
    expect(translate("deDE", "stack.health.skippedVms", { count: 2 })).toContain("2 gestoppte VM(s)");
  });
});
