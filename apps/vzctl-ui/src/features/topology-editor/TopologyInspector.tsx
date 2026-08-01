import { useMemo, useState } from "react";
import { useEditorStore } from "@/store/editorStore";
import { PolicyRuleEditor } from "@/features/firewall-rules/PolicyRuleEditor";
import { NameField } from "@/features/topology-editor/NameField";
import { formatValidationIssue } from "@/application/validation/formatIssue";
import type { ValidationIssue } from "@/application/validation/topology";
import { useT } from "@/lib/i18n";

export function TopologyInspector() {
  const t = useT();
  const env = useEditorStore((s) => s.env);
  const selection = useEditorStore((s) => s.selection);
  const validation = useEditorStore((s) => s.validation);
  const updateVm = useEditorStore((s) => s.updateVm);
  const renameVm = useEditorStore((s) => s.renameVm);
  const updateNetwork = useEditorStore((s) => s.updateNetwork);
  const renameNetwork = useEditorStore((s) => s.renameNetwork);
  const updateNicIp = useEditorStore((s) => s.updateNicIp);
  const attachNic = useEditorStore((s) => s.attachNic);
  const detachNic = useEditorStore((s) => s.detachNic);
  const ensurePolicy = useEditorStore((s) => s.ensurePolicy);
  const setAllowRules = useEditorStore((s) => s.setAllowRules);
  const upsertRoute = useEditorStore((s) => s.upsertRoute);
  const deleteRoute = useEditorStore((s) => s.deleteRoute);
  const setSelection = useEditorStore((s) => s.setSelection);

  const selectedNodeId = selection.nodeIds[0];
  const selectedEdgeId = selection.edgeIds[0];

  const issuesForSelection = useMemo(() => {
    if (!selectedNodeId && !selectedEdgeId) return validation;
    return validation.filter(
      (i) =>
        i.nodeId === selectedNodeId ||
        i.connectionId === selectedEdgeId ||
        (selectedNodeId?.startsWith("net:") &&
          i.policyName &&
          env?.spec.policies.some(
            (p) =>
              p.name === i.policyName &&
              p.network === selectedNodeId.slice(4),
          )),
    );
  }, [validation, selectedNodeId, selectedEdgeId, env]);

  if (!env) {
    return (
      <aside className="topology-inspector" aria-label={t("topo.inspector")}>
        <h3 className="topology-panel-title">{t("topo.inspector")}</h3>
        <p className="muted">{t("topo.noProject")}</p>
      </aside>
    );
  }

  const networkNames = Object.keys(env.spec.networks);

  if (selectedNodeId?.startsWith("vm:")) {
    const name = selectedNodeId.slice(3);
    const vm = env.spec.vms[name];
    if (!vm) return <EmptyInspector />;
    const unusedNets = networkNames.filter(
      (n) => !vm.networks.some((x) => x.name === n),
    );
    return (
      <aside className="topology-inspector" aria-label={t("topo.inspector")}>
        <h3 className="topology-panel-title">{t("topo.vmTitle", { name })}</h3>
        <NameField value={name} onCommit={(next) => renameVm(name, next)} />
        <label className="topology-field">
          <span>{t("topo.field.cpus")}</span>
          <input
            type="number"
            min={1}
            value={vm.cpus ?? 2}
            onChange={(e) =>
              updateVm(name, { cpus: Number(e.target.value) || 1 })
            }
          />
        </label>
        <label className="topology-field">
          <span>{t("topo.field.memory")}</span>
          <input
            type="text"
            value={String(vm.memory ?? "2048MiB")}
            onChange={(e) => updateVm(name, { memory: e.target.value })}
          />
        </label>
        <label className="topology-field">
          <span>{t("topo.field.dataDisk")}</span>
          <input
            type="text"
            value={vm.dataDisk}
            onChange={(e) => updateVm(name, { dataDisk: e.target.value })}
          />
        </label>
        <fieldset className="topology-fieldset">
          <legend>{t("topo.field.roles")}</legend>
          {(["router", "docker"] as const).map((role) => (
            <label key={role} className="topology-check">
              <input
                type="checkbox"
                checked={vm.roles.includes(role)}
                onChange={(e) => {
                  const roles = e.target.checked
                    ? [...vm.roles, role]
                    : vm.roles.filter((r) => r !== role);
                  updateVm(name, { roles });
                }}
              />
              {role}
            </label>
          ))}
        </fieldset>
        <h4>{t("topo.interfaces")}</h4>
        <ul className="topology-nic-list">
          {vm.networks.map((nic) => (
            <li key={nic.name}>
              <strong>{nic.name}</strong>
              <input
                aria-label={t("topo.ipFor", { name: nic.name })}
                value={nic.ip}
                onChange={(e) => updateNicIp(name, nic.name, e.target.value)}
              />
              <button
                type="button"
                className="secondary"
                disabled={vm.networks.length <= 1}
                onClick={() => detachNic(name, nic.name)}
              >
                {t("topo.detach")}
              </button>
            </li>
          ))}
        </ul>
        {unusedNets.length > 0 ? (
          <label className="topology-field">
            <span>{t("topo.attachNet")}</span>
            <select
              defaultValue=""
              onChange={(e) => {
                if (e.target.value) attachNic(name, e.target.value);
                e.target.value = "";
              }}
            >
              <option value="">{t("topo.attachSelect")}</option>
              {unusedNets.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        <IssuesList
          issues={issuesForSelection}
          onFocus={(nodeId) =>
            setSelection({ nodeIds: nodeId ? [nodeId] : [], edgeIds: [] })
          }
        />
      </aside>
    );
  }

  if (selectedNodeId?.startsWith("net:")) {
    const name = selectedNodeId.slice(4);
    const net = env.spec.networks[name];
    if (!net) return <EmptyInspector />;
    const policy =
      env.spec.policies.find((p) => p.network === name) ?? null;
    return (
      <aside className="topology-inspector" aria-label={t("topo.inspector")}>
        <h3 className="topology-panel-title">{t("topo.netTitle", { name })}</h3>
        <NameField
          value={name}
          onCommit={(next) => renameNetwork(name, next)}
        />
        <label className="topology-field">
          <span>{t("topo.field.cidr")}</span>
          <input
            value={net.cidr}
            onChange={(e) => updateNetwork(name, { cidr: e.target.value })}
          />
        </label>
        <label className="topology-field">
          <span>{t("topo.field.mode")}</span>
          <select
            value={net.mode}
            onChange={(e) =>
              updateNetwork(name, {
                mode: e.target.value as "shared" | "host",
              })
            }
          >
            <option value="shared">{t("topo.mode.shared")}</option>
            <option value="host">{t("topo.mode.host")}</option>
          </select>
        </label>
        <label className="topology-field">
          <span>{t("topo.field.backend")}</span>
          <select
            value={net.backend ?? "vmnet"}
            onChange={(e) =>
              updateNetwork(name, {
                backend: e.target.value as "vmnet" | "docker",
              })
            }
          >
            <option value="vmnet">{t("topo.backend.vmnet")}</option>
            <option value="docker">{t("topo.backend.docker")}</option>
          </select>
        </label>
        {net.backend === "docker" ? (
          <p className="muted">{t("topo.dockerBackendHint")}</p>
        ) : null}
        <label className="topology-check">
          <input
            type="checkbox"
            checked={net.dhcp}
            disabled={net.backend === "docker"}
            onChange={(e) => updateNetwork(name, { dhcp: e.target.checked })}
          />
          {t("topo.field.dhcp")}
        </label>
        <label className="topology-check">
          <input
            type="checkbox"
            checked={net.natEgress !== false}
            disabled={net.backend === "docker"}
            onChange={(e) =>
              updateNetwork(name, { natEgress: e.target.checked })
            }
          />
          {t("topo.field.natEgress")}
        </label>
        <h4>{t("topo.firewallPolicy")}</h4>
        {!policy ? (
          <button type="button" onClick={() => ensurePolicy(name)}>
            {t("topo.createPolicy")}
          </button>
        ) : (
          <PolicyRuleEditor
            policyName={policy.name}
            networkName={name}
            networks={networkNames}
            allow={policy.allow}
            onChange={(allow) => setAllowRules(policy.name, allow)}
          />
        )}
        <RouteSection
          networkName={name}
          env={env}
          onUpsert={upsertRoute}
          onDelete={deleteRoute}
        />
        <IssuesList
          issues={issuesForSelection}
          onFocus={(nodeId) =>
            setSelection({ nodeIds: nodeId ? [nodeId] : [], edgeIds: [] })
          }
        />
      </aside>
    );
  }

  if (selectedNodeId?.startsWith("igw:")) {
    return (
      <aside className="topology-inspector" aria-label={t("topo.inspector")}>
        <h3 className="topology-panel-title">{t("topo.igwTitle")}</h3>
        <p className="muted">
          {t("topo.igwHint", { net: selectedNodeId.slice(4) })}
        </p>
      </aside>
    );
  }

  return (
    <aside className="topology-inspector" aria-label={t("topo.inspector")}>
      <h3 className="topology-panel-title">{t("topo.inspector")}</h3>
      <p className="muted">{t("topo.selectHint")}</p>
      <IssuesList
        issues={validation}
        onFocus={(nodeId) =>
          setSelection({ nodeIds: nodeId ? [nodeId] : [], edgeIds: [] })
        }
      />
    </aside>
  );
}

function EmptyInspector() {
  const t = useT();
  return (
    <aside className="topology-inspector" aria-label={t("topo.inspector")}>
      <p className="muted">{t("topo.invalidSelection")}</p>
    </aside>
  );
}

function IssuesList({
  issues,
  onFocus,
}: {
  issues: ValidationIssue[];
  onFocus: (nodeId?: string) => void;
}) {
  const t = useT();
  if (issues.length === 0) {
    return (
      <div className="topology-issues">
        <h4>{t("topo.validation")}</h4>
        <p className="muted">{t("topo.noIssues")}</p>
      </div>
    );
  }
  return (
    <div className="topology-issues">
      <h4>{t("topo.validationCount", { n: issues.length })}</h4>
      <ul>
        {issues.map((issue) => (
          <li key={issue.id}>
            <button
              type="button"
              className={`issue-item severity-${issue.severity}`}
              onClick={() => onFocus(issue.nodeId)}
            >
              <span className="issue-sev">{issue.severity}</span>
              {formatValidationIssue(issue, t)}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function RouteSection({
  networkName,
  env,
  onUpsert,
  onDelete,
}: {
  networkName: string;
  env: NonNullable<ReturnType<typeof useEditorStore.getState>["env"]>;
  onUpsert: (r: { name: string; from: string; to: string; via: string }) => void;
  onDelete: (name: string) => void;
}) {
  const t = useT();
  const routes = env.spec.routes.filter(
    (r) => r.from === networkName || r.to === networkName,
  );
  const routers = Object.entries(env.spec.vms)
    .filter(([, vm]) => vm.roles.includes("router"))
    .map(([n]) => n);
  const nets = Object.keys(env.spec.networks);
  const [to, setTo] = useState(nets.find((n) => n !== networkName) ?? "");
  const [via, setVia] = useState(routers[0] ?? "");

  return (
    <div className="topology-routes">
      <h4>{t("topo.routes")}</h4>
      {routes.length === 0 ? (
        <p className="muted">{t("topo.noRoutes")}</p>
      ) : (
        <ul>
          {routes.map((r) => (
            <li key={r.name} className="row" style={{ gap: "0.5rem" }}>
              <code>
                {r.name}: {r.from}→{r.to} via {r.via}
              </code>
              <button
                type="button"
                className="secondary"
                onClick={() => onDelete(r.name)}
              >
                {t("topo.routeDelete")}
              </button>
            </li>
          ))}
        </ul>
      )}
      {routers.length > 0 && nets.length > 1 ? (
        <div className="topology-route-form">
          <label className="topology-field">
            <span>{t("topo.routeTarget")}</span>
            <select value={to} onChange={(e) => setTo(e.target.value)}>
              {nets
                .filter((n) => n !== networkName)
                .map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
            </select>
          </label>
          <label className="topology-field">
            <span>{t("topo.routeVia")}</span>
            <select value={via} onChange={(e) => setVia(e.target.value)}>
              {routers.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={!to || !via}
            onClick={() =>
              onUpsert({
                name: `${networkName}-to-${to}`,
                from: networkName,
                to,
                via,
              })
            }
          >
            {t("topo.routeCreate")}
          </button>
        </div>
      ) : (
        <p className="muted">{t("topo.routeNeedsRouter")}</p>
      )}
    </div>
  );
}
