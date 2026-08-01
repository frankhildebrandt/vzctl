import type { Environment } from "@/domain/hypernetwork/schema";
import { emptyDiagramState } from "@/domain/diagram/types";
import type { EditorSnapshot } from "@/application/commands/mutations";
import {
  applyAttachNic as attach,
  applyAssignPrimaryNetwork as assignPrimary,
  applyCreateNetwork as createNet,
  applyCreateVm as createVm,
  applyDeleteNodes as delNodes,
  applyReassignNic as reassign,
  applyRenameNetwork as renameNet,
  applyRenameVm as renameVm,
} from "@/application/commands/mutations";

export function emptySnapshot(): EditorSnapshot {
  const env: Environment = {
    apiVersion: "hypernetwork/v1",
    kind: "Environment",
    metadata: { name: "test" },
    spec: {
      project: "test",
      domain: "test.vz.test",
      dns: {
        enabled: true,
        hostResolver: true,
        hostListen: "127.0.0.1:15353",
        forward: { enabled: true, upstream: "system" },
      },
      images: { "ubuntu-base": { from: "ubuntu-latest", role: "base", tag: "v1" } },
      networks: {},
      routes: [],
      policies: [],
      ports: [],
      volumes: {},
      vms: {},
    },
  };
  return { env, diagram: emptyDiagramState() };
}

export const applyCreateNetwork = createNet;
export const applyCreateVm = createVm;
export const applyAttachNic = attach;
export const applyAssignPrimaryNetwork = assignPrimary;
export const applyReassignNic = reassign;
export const applyDeleteNodes = delNodes;
export const applyRenameVm = renameVm;
export const applyRenameNetwork = renameNet;
