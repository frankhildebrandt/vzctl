import { api } from "@/lib/api";
import { loadProject, saveProject } from "@/features/persistence/projectIo";
import { cidrsOverlap } from "@/application/validation/topology";
import {
  findNetworksByCidr,
  remapNetworkCidr,
  suggestReplacementCidr,
  type VmnetOrphanInfo,
} from "@/lib/vmnetOrphan";

export async function requestHostReboot(): Promise<void> {
  await api.post("/v1/host/reboot");
}

export async function recoverOrphanByCidrChange(
  projectDir: string,
  orphan: VmnetOrphanInfo,
  preferredCidr?: string,
): Promise<{ networkNames: string[]; newCidr: string }> {
  const { env, diagram } = await loadProject(projectDir);
  const names = findNetworksByCidr(env, orphan.cidr);
  if (names.length === 0) {
    throw new Error(
      `Kein Netz mit CIDR ${orphan.cidr} in hypernetwork.config.yaml`,
    );
  }
  const used = Object.values(env.spec.networks).map((n) => n.cidr);
  const newCidr =
    preferredCidr &&
    preferredCidr !== orphan.cidr &&
    !used.some((c) => c === preferredCidr || cidrsOverlap(c, preferredCidr))
      ? preferredCidr
      : suggestReplacementCidr(orphan.cidr, used);
  let next = env;
  for (const name of names) {
    next = remapNetworkCidr(next, name, newCidr);
  }
  await saveProject(projectDir, next, diagram);
  return { networkNames: names, newCidr };
}
