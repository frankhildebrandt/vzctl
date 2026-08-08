import { create } from "zustand";
import type { Environment, AllowRule, NetworkMode } from "@/domain/hypernetwork/schema";
import {
  emptyDiagramState,
  type DiagramState,
} from "@/domain/diagram/types";
import type { ConnectionHint } from "@/application/commands/mutations";
import type { ValidationIssue } from "@/application/validation/topology";
import {
  applyAttachNic,
  applyAssignPrimaryNetwork,
  applyCreateNetwork,
  applyCreateVm,
  applyDeleteNodes,
  applyDeletePolicy,
  applyDeleteRoute,
  applyDetachNic,
  applyEnsurePolicyForNetwork,
  applyMoveNode,
  applyMoveNodes,
  applyReassignNic,
  applyResizeNode,
  applySetAllowRules,
  applyUpdateNetwork,
  applyUpdateNicIp,
  applyUpdateVm,
  applyRenameNetwork,
  applyRenameVm,
  applyUpsertPolicy,
  applyUpsertRoute,
  revalidate,
  type EditorSnapshot,
  type SelectionState,
  type EditorUiState,
} from "@/application/commands/mutations";
import { layoutByNetwork } from "@/diagram/projections/layout";

const HISTORY_LIMIT = 80;

type EditorStore = {
  projectPath: string | null;
  env: Environment | null;
  diagram: DiagramState;
  selection: SelectionState;
  validation: ValidationIssue[];
  ui: EditorUiState;
  past: EditorSnapshot[];
  future: EditorSnapshot[];

  load: (path: string, env: Environment, diagram: DiagramState) => void;
  reset: () => void;
  setSelection: (selection: SelectionState) => void;
  setPaletteFilter: (filter: string) => void;
  setConnectionHint: (hint: ConnectionHint) => void;
  setProjecting: (value: boolean) => void;
  setLastError: (error: string | null) => void;
  markSaved: () => void;

  snapshot: () => EditorSnapshot | null;
  pushHistory: (before: EditorSnapshot) => void;
  undo: () => void;
  redo: () => void;

  createNetwork: (
    name: string,
    cidr: string,
    mode: NetworkMode,
    position: { x: number; y: number },
  ) => void;
  createVm: (
    name: string,
    opts: {
      networkName?: string;
      roles?: Array<"router" | "docker">;
      position: { x: number; y: number };
    },
  ) => void;
  deleteSelection: () => void;
  attachNic: (vmName: string, networkName: string, ip?: string) => void;
  detachNic: (vmName: string, networkName: string) => void;
  reassignNic: (
    vmName: string,
    fromNetwork: string,
    toNetwork: string,
  ) => void;
  assignPrimaryNetwork: (
    vmName: string,
    networkName: string,
    position?: { x: number; y: number },
  ) => void;
  moveNode: (nodeId: string, x: number, y: number) => void;
  moveNodes: (
    positions: Array<{ nodeId: string; x: number; y: number }>,
  ) => void;
  resizeNode: (nodeId: string, width: number, height: number) => void;
  updateVm: (
    name: string,
    patch: Partial<{
      cpus: number;
      memory: string;
      disk: string;
      roles: Array<"router" | "docker">;
      dependsOn: string[];
    }>,
  ) => void;
  renameVm: (oldName: string, newName: string) => void;
  updateNetwork: (
    name: string,
    patch: Partial<{
      cidr: string;
      mode: NetworkMode;
      dhcp: boolean;
      natEgress: boolean;
      backend: "vmnet" | "docker";
    }>,
  ) => void;
  renameNetwork: (oldName: string, newName: string) => void;
  updateNicIp: (vmName: string, networkName: string, ip: string) => void;
  upsertRoute: (route: {
    name: string;
    from: string;
    to: string;
    via: string;
  }) => void;
  deleteRoute: (name: string) => void;
  ensurePolicy: (networkName: string) => void;
  setAllowRules: (policyName: string, allow: AllowRule[]) => void;
  upsertPolicy: (policy: {
    name: string;
    network: string;
    allow: AllowRule[];
  }) => void;
  deletePolicy: (name: string) => void;
  replaceDiagram: (diagram: DiagramState) => void;
};

function defaultUi(): EditorUiState {
  return {
    dirty: false,
    projecting: false,
    paletteFilter: "",
    lastError: null,
    connectionHint: null,
  };
}

function applyMutation(
  set: (fn: (s: EditorStore) => Partial<EditorStore>) => void,
  get: () => EditorStore,
  mutate: (snap: EditorSnapshot) => EditorSnapshot,
): void {
  const state = get();
  if (!state.env) return;
  const before: EditorSnapshot = {
    env: structuredClone(state.env),
    diagram: structuredClone(state.diagram),
  };
  try {
    const next = mutate(before);
    const past = [...state.past, before].slice(-HISTORY_LIMIT);
    set(() => ({
      env: next.env,
      diagram: next.diagram,
      validation: revalidate(next.env),
      past,
      future: [],
      ui: { ...state.ui, dirty: true, lastError: null },
    }));
  } catch (err) {
    set(() => ({
      ui: { ...state.ui, lastError: String(err instanceof Error ? err.message : err) },
    }));
  }
}

export const useEditorStore = create<EditorStore>((set, get) => ({
  projectPath: null,
  env: null,
  diagram: emptyDiagramState(),
  selection: { nodeIds: [], edgeIds: [] },
  validation: [],
  ui: defaultUi(),
  past: [],
  future: [],

  load: (path, env, diagram) => {
    const laidOut = layoutByNetwork(env);
    const nextDiagram: DiagramState = {
      ...diagram,
      nodes: { ...diagram.nodes, ...laidOut },
    };
    set({
      projectPath: path,
      env,
      diagram: nextDiagram,
      selection: { nodeIds: [], edgeIds: [] },
      validation: revalidate(env),
      ui: { ...defaultUi(), dirty: false },
      past: [],
      future: [],
    });
  },

  reset: () =>
    set({
      projectPath: null,
      env: null,
      diagram: emptyDiagramState(),
      selection: { nodeIds: [], edgeIds: [] },
      validation: [],
      ui: defaultUi(),
      past: [],
      future: [],
    }),

  setSelection: (selection) => set({ selection }),
  setPaletteFilter: (paletteFilter) =>
    set((s) => ({ ui: { ...s.ui, paletteFilter } })),
  setConnectionHint: (connectionHint) =>
    set((s) => ({ ui: { ...s.ui, connectionHint } })),
  setProjecting: (projecting) =>
    set((s) => ({ ui: { ...s.ui, projecting } })),
  setLastError: (lastError) => set((s) => ({ ui: { ...s.ui, lastError } })),
  markSaved: () => set((s) => ({ ui: { ...s.ui, dirty: false } })),

  snapshot: () => {
    const s = get();
    if (!s.env) return null;
    return { env: s.env, diagram: s.diagram };
  },

  pushHistory: (before) =>
    set((s) => ({
      past: [...s.past, before].slice(-HISTORY_LIMIT),
      future: [],
    })),

  undo: () => {
    const s = get();
    const prev = s.past[s.past.length - 1];
    if (!prev || !s.env) return;
    const current: EditorSnapshot = {
      env: structuredClone(s.env),
      diagram: structuredClone(s.diagram),
    };
    set({
      env: prev.env,
      diagram: prev.diagram,
      validation: revalidate(prev.env),
      past: s.past.slice(0, -1),
      future: [current, ...s.future],
      ui: { ...s.ui, dirty: true },
    });
  },

  redo: () => {
    const s = get();
    const next = s.future[0];
    if (!next || !s.env) return;
    const current: EditorSnapshot = {
      env: structuredClone(s.env),
      diagram: structuredClone(s.diagram),
    };
    set({
      env: next.env,
      diagram: next.diagram,
      validation: revalidate(next.env),
      past: [...s.past, current],
      future: s.future.slice(1),
      ui: { ...s.ui, dirty: true },
    });
  },

  createNetwork: (name, cidr, mode, position) =>
    applyMutation(set, get, (snap) =>
      applyCreateNetwork(snap, name, cidr, mode, position),
    ),
  createVm: (name, opts) =>
    applyMutation(set, get, (snap) => applyCreateVm(snap, name, opts)),
  deleteSelection: () => {
    const { selection } = get();
    const ids = [...selection.nodeIds];
    // Edge deletes: detach / delete route
    for (const edgeId of selection.edgeIds) {
      if (edgeId.startsWith("attach:")) {
        const parts = edgeId.split(":");
        const vmName = parts[1];
        const networkName = parts[2];
        if (vmName && networkName) {
          applyMutation(set, get, (snap) =>
            applyDetachNic(snap, vmName, networkName),
          );
        }
      } else if (edgeId.startsWith("route:")) {
        applyMutation(set, get, (snap) =>
          applyDeleteRoute(snap, edgeId.slice(6)),
        );
      }
    }
    if (ids.length > 0) {
      applyMutation(set, get, (snap) => applyDeleteNodes(snap, ids));
    }
    set({ selection: { nodeIds: [], edgeIds: [] } });
  },
  attachNic: (vmName, networkName, ip) =>
    applyMutation(set, get, (snap) =>
      applyAttachNic(snap, vmName, networkName, ip),
    ),
  detachNic: (vmName, networkName) =>
    applyMutation(set, get, (snap) =>
      applyDetachNic(snap, vmName, networkName),
    ),
  reassignNic: (vmName, fromNetwork, toNetwork) =>
    applyMutation(set, get, (snap) =>
      applyReassignNic(snap, vmName, fromNetwork, toNetwork),
    ),
  assignPrimaryNetwork: (vmName, networkName, position) =>
    applyMutation(set, get, (snap) =>
      applyAssignPrimaryNetwork(snap, vmName, networkName, position),
    ),
  moveNode: (nodeId, x, y) =>
    applyMutation(set, get, (snap) => applyMoveNode(snap, nodeId, x, y)),
  moveNodes: (positions) =>
    applyMutation(set, get, (snap) => applyMoveNodes(snap, positions)),
  resizeNode: (nodeId, width, height) =>
    applyMutation(set, get, (snap) =>
      applyResizeNode(snap, nodeId, width, height),
    ),
  updateVm: (name, patch) =>
    applyMutation(set, get, (snap) => applyUpdateVm(snap, name, patch)),
  renameVm: (oldName, newName) => {
    applyMutation(set, get, (snap) => applyRenameVm(snap, oldName, newName));
    const state = get();
    if (state.env?.spec.vms[newName.trim()]) {
      set({
        selection: { nodeIds: [`vm:${newName.trim()}`], edgeIds: [] },
      });
    }
  },
  updateNetwork: (name, patch) =>
    applyMutation(set, get, (snap) => applyUpdateNetwork(snap, name, patch)),
  renameNetwork: (oldName, newName) => {
    applyMutation(set, get, (snap) =>
      applyRenameNetwork(snap, oldName, newName),
    );
    const trimmed = newName.trim();
    const state = get();
    if (state.env?.spec.networks[trimmed]) {
      set({
        selection: { nodeIds: [`net:${trimmed}`], edgeIds: [] },
      });
    }
  },
  updateNicIp: (vmName, networkName, ip) =>
    applyMutation(set, get, (snap) =>
      applyUpdateNicIp(snap, vmName, networkName, ip),
    ),
  upsertRoute: (route) =>
    applyMutation(set, get, (snap) => applyUpsertRoute(snap, route)),
  deleteRoute: (name) =>
    applyMutation(set, get, (snap) => applyDeleteRoute(snap, name)),
  ensurePolicy: (networkName) =>
    applyMutation(set, get, (snap) =>
      applyEnsurePolicyForNetwork(snap, networkName),
    ),
  setAllowRules: (policyName, allow) =>
    applyMutation(set, get, (snap) =>
      applySetAllowRules(snap, policyName, allow),
    ),
  upsertPolicy: (policy) =>
    applyMutation(set, get, (snap) => applyUpsertPolicy(snap, policy)),
  deletePolicy: (name) =>
    applyMutation(set, get, (snap) => applyDeletePolicy(snap, name)),
  replaceDiagram: (diagram) =>
    set((s) => ({
      diagram,
      ui: { ...s.ui, dirty: true },
    })),
}));
