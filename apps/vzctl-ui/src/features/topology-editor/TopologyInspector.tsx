import { useMemo, useState } from "react";
import { useEditorStore } from "@/store/editorStore";
import { PolicyRuleEditor } from "@/features/firewall-rules/PolicyRuleEditor";
import { NameField } from "@/features/topology-editor/NameField";

export function TopologyInspector() {
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
      <aside className="topology-inspector" aria-label="Inspector">
        <h3 className="topology-panel-title">Inspector</h3>
        <p className="muted">Kein Projekt geladen.</p>
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
      <aside className="topology-inspector" aria-label="Inspector">
        <h3 className="topology-panel-title">VM · {name}</h3>
        <NameField value={name} onCommit={(next) => renameVm(name, next)} />
        <label className="topology-field">
          <span>CPUs</span>
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
          <span>Memory</span>
          <input
            type="text"
            value={String(vm.memory ?? "2048MiB")}
            onChange={(e) => updateVm(name, { memory: e.target.value })}
          />
        </label>
        <label className="topology-field">
          <span>dataDisk</span>
          <input
            type="text"
            value={vm.dataDisk}
            onChange={(e) => updateVm(name, { dataDisk: e.target.value })}
          />
        </label>
        <fieldset className="topology-fieldset">
          <legend>Roles</legend>
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
        <h4>Interfaces</h4>
        <ul className="topology-nic-list">
          {vm.networks.map((nic) => (
            <li key={nic.name}>
              <strong>{nic.name}</strong>
              <input
                aria-label={`IP für ${nic.name}`}
                value={nic.ip}
                onChange={(e) => updateNicIp(name, nic.name, e.target.value)}
              />
              <button
                type="button"
                className="secondary"
                disabled={vm.networks.length <= 1}
                onClick={() => detachNic(name, nic.name)}
              >
                Trennen
              </button>
            </li>
          ))}
        </ul>
        {unusedNets.length > 0 ? (
          <label className="topology-field">
            <span>Netz anhängen</span>
            <select
              defaultValue=""
              onChange={(e) => {
                if (e.target.value) attachNic(name, e.target.value);
                e.target.value = "";
              }}
            >
              <option value="">— wählen —</option>
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
      <aside className="topology-inspector" aria-label="Inspector">
        <h3 className="topology-panel-title">Netz · {name}</h3>
        <NameField
          value={name}
          onCommit={(next) => renameNetwork(name, next)}
        />
        <label className="topology-field">
          <span>CIDR</span>
          <input
            value={net.cidr}
            onChange={(e) => updateNetwork(name, { cidr: e.target.value })}
          />
        </label>
        <label className="topology-field">
          <span>Mode</span>
          <select
            value={net.mode}
            onChange={(e) =>
              updateNetwork(name, {
                mode: e.target.value as "shared" | "host",
              })
            }
          >
            <option value="shared">shared</option>
            <option value="host">host</option>
          </select>
        </label>
        <label className="topology-check">
          <input
            type="checkbox"
            checked={net.dhcp}
            onChange={(e) => updateNetwork(name, { dhcp: e.target.checked })}
          />
          DHCP
        </label>
        <label className="topology-check">
          <input
            type="checkbox"
            checked={net.natEgress !== false}
            onChange={(e) =>
              updateNetwork(name, { natEgress: e.target.checked })
            }
          />
          Internet (NAT-Egress)
        </label>
        <h4>Firewall-Policy</h4>
        {!policy ? (
          <button type="button" onClick={() => ensurePolicy(name)}>
            Policy anlegen
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
      <aside className="topology-inspector" aria-label="Inspector">
        <h3 className="topology-panel-title">Internet (Wolke)</h3>
        <p className="muted">
          Visuell: Host-Gateway <code>.0</code> auf Netz{" "}
          <code>{selectedNodeId.slice(4)}</code>. Nicht in YAML gespeichert.
        </p>
      </aside>
    );
  }

  return (
    <aside className="topology-inspector" aria-label="Inspector">
      <h3 className="topology-panel-title">Inspector</h3>
      <p className="muted">Element auswählen, um Eigenschaften zu bearbeiten.</p>
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
  return (
    <aside className="topology-inspector" aria-label="Inspector">
      <p className="muted">Auswahl ungültig.</p>
    </aside>
  );
}

function IssuesList({
  issues,
  onFocus,
}: {
  issues: Array<{
    id: string;
    severity: string;
    message: string;
    nodeId?: string;
  }>;
  onFocus: (nodeId?: string) => void;
}) {
  if (issues.length === 0) {
    return (
      <div className="topology-issues">
        <h4>Validierung</h4>
        <p className="muted">Keine Probleme.</p>
      </div>
    );
  }
  return (
    <div className="topology-issues">
      <h4>Validierung ({issues.length})</h4>
      <ul>
        {issues.map((issue) => (
          <li key={issue.id}>
            <button
              type="button"
              className={`issue-item severity-${issue.severity}`}
              onClick={() => onFocus(issue.nodeId)}
            >
              <span className="issue-sev">{issue.severity}</span>
              {issue.message}
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
      <h4>Routes</h4>
      {routes.length === 0 ? (
        <p className="muted">Keine Routes für dieses Netz.</p>
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
                Löschen
              </button>
            </li>
          ))}
        </ul>
      )}
      {routers.length > 0 && nets.length > 1 ? (
        <div className="topology-route-form">
          <label className="topology-field">
            <span>Zielnetz</span>
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
            <span>Via Router</span>
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
            Route anlegen
          </button>
        </div>
      ) : (
        <p className="muted">Route braucht Router-VM und ≥2 Netze.</p>
      )}
    </div>
  );
}
