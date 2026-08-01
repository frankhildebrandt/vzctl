import type { Project } from "@/lib/projects";
import {
  parseStackInventory,
  type StackInventory,
} from "@/lib/stackStatus";
import type { VmListItem } from "@/lib/vms";

export type ActiveStack = {
  project: Project;
  vmIds: string[];
};

/** Stacks that have at least one existing VM, plus those VM ids. */
export function partitionStacksAndVms(input: {
  projects: Project[];
  vms: VmListItem[];
  statusByPath: Record<string, string | undefined>;
}): {
  activeStacks: ActiveStack[];
  standaloneVms: VmListItem[];
} {
  const { projects, vms, statusByPath } = input;
  const stackedIds = new Set<string>();
  const activeStacks: ActiveStack[] = [];

  for (const project of projects) {
    const inventory = parseStackInventory(statusByPath[project.path] ?? null);
    const vmIds = stackVmIds(project, inventory, vms);
    if (vmIds.length === 0) continue;
    for (const id of vmIds) stackedIds.add(id);
    activeStacks.push({ project, vmIds });
  }

  return {
    activeStacks,
    standaloneVms: vms.filter((vm) => !stackedIds.has(vm.id)),
  };
}

function stackVmIds(
  project: Project,
  inventory: StackInventory | null,
  vms: VmListItem[],
): string[] {
  const keys = projectKeys(project, inventory);
  const fromInventory = (inventory?.items ?? [])
    .filter((item) => item.present !== false && item.state !== "missing")
    .map((item) => item.id);

  const matched = vms.filter((vm) => {
    if (fromInventory.includes(vm.id)) return true;
    return keys.some(
      (key) => vm.id === key || vm.id.startsWith(`${key}/`),
    );
  });

  return matched.map((vm) => vm.id);
}

function projectKeys(
  project: Project,
  inventory: StackInventory | null,
): string[] {
  const keys = new Set<string>();
  keys.add(project.name);
  if (typeof inventory?.project === "string" && inventory.project) {
    keys.add(inventory.project);
  }
  if (typeof inventory?.stack_id === "string" && inventory.stack_id) {
    const stackId = inventory.stack_id;
    const colon = stackId.indexOf(":");
    if (colon > 0) keys.add(stackId.slice(0, colon));
  }
  return [...keys];
}
