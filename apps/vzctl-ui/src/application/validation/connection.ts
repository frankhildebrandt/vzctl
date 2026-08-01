import type { Environment } from "@/domain/hypernetwork/schema";
import type { MessageKey, MessageParams } from "@/lib/i18n";
import {
  isNetworkAttachPortId,
  isVmNicPortId,
} from "@/domain/hypernetwork/ids";

export type ConnectionEndpoint = {
  nodeId: string;
  portId: string;
};

export type ConnectionAttempt = {
  source: ConnectionEndpoint;
  target: ConnectionEndpoint;
  /** Existing attachments as `vmName|networkName`. */
  existingAttachments: Set<string>;
  /** Existing route keys `from|to|via`. */
  existingRoutes: Set<string>;
  env: Environment;
  /** When reconnecting an attachment, exclude this pair from the duplicate check. */
  ignoreAttachment?: string;
};

export type ConnectionValidationResult =
  | {
      ok: true;
      kind: "attachment" | "route";
      vmName?: string;
      networkName?: string;
      from?: string;
      to?: string;
      via?: string;
    }
  | { ok: false; reasonKey: MessageKey; reasonParams?: MessageParams };

function fail(
  reasonKey: MessageKey,
  reasonParams?: MessageParams,
): Extract<ConnectionValidationResult, { ok: false }> {
  return { ok: false, reasonKey, reasonParams };
}

function parseVm(id: string): string | null {
  return id.startsWith("vm:") ? id.slice(3) : null;
}

function parseNet(id: string): string | null {
  return id.startsWith("net:") ? id.slice(4) : null;
}

function portsAllowAttachment(source: ConnectionEndpoint, target: ConnectionEndpoint): boolean {
  const sp = source.portId;
  const tp = target.portId;
  // Allow legacy empty ports (node-level) for tests / fallbacks
  if (!sp && !tp) return true;
  const srcVmNic = isVmNicPortId(sp);
  const tgtVmNic = isVmNicPortId(tp);
  const srcAttach = isNetworkAttachPortId(sp);
  const tgtAttach = isNetworkAttachPortId(tp);
  return (srcVmNic && tgtAttach) || (srcAttach && tgtVmNic);
}

/**
 * Fachliche Verbindungsregeln (unabhängig von X6).
 * Erlaubt: VM-NIC-Port ↔ Network-Attach-Port.
 */
export function validateConnection(
  attempt: ConnectionAttempt,
): ConnectionValidationResult {
  const { source, target, existingAttachments, env, ignoreAttachment } =
    attempt;

  if (source.nodeId === target.nodeId) {
    return fail("conn.selfLink");
  }

  const srcVm = parseVm(source.nodeId);
  const tgtVm = parseVm(target.nodeId);
  const srcNet = parseNet(source.nodeId);
  const tgtNet = parseNet(target.nodeId);

  if (srcVm && tgtNet) {
    if (!portsAllowAttachment(source, target)) {
      return fail("conn.nicPortOnly");
    }
    return validateAttachment(
      srcVm,
      tgtNet,
      existingAttachments,
      env,
      ignoreAttachment,
    );
  }
  if (tgtVm && srcNet) {
    if (!portsAllowAttachment(source, target)) {
      return fail("conn.nicPortOnly");
    }
    return validateAttachment(
      tgtVm,
      srcNet,
      existingAttachments,
      env,
      ignoreAttachment,
    );
  }

  if (srcNet && tgtNet) {
    return fail("conn.netToNetRoute");
  }

  if (srcVm && tgtVm) {
    return fail("conn.vmDirect");
  }

  return fail("conn.invalidEndpoints");
}

function validateAttachment(
  vmName: string,
  networkName: string,
  existing: Set<string>,
  env: Environment,
  ignoreAttachment?: string,
): ConnectionValidationResult {
  const vm = env.spec.vms[vmName];
  const net = env.spec.networks[networkName];
  if (!vm) return fail("conn.vmMissing", { name: vmName });
  if (!net) return fail("conn.netMissing", { name: networkName });

  const key = `${vmName}|${networkName}`;
  if (key !== ignoreAttachment && existing.has(key)) {
    return fail("conn.attachmentExists");
  }

  return {
    ok: true,
    kind: "attachment",
    vmName,
    networkName,
  };
}
