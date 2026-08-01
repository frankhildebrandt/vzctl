import { Link } from "@tanstack/react-router";
import type { StackVmItem } from "@/lib/stackStatus";
import { encodeVmIdParam } from "@/lib/vms";

function canOpenVm(item: StackVmItem): boolean {
  return item.present !== false && item.state !== "missing";
}

/** Config-/Kurzname ohne Project-Prefix (`edge/web` → `web`). */
function stackVmLabel(item: StackVmItem): string {
  if (item.name) return item.name;
  const slash = item.id.lastIndexOf("/");
  return slash >= 0 ? item.id.slice(slash + 1) : item.id;
}

export function StackVmList({
  items,
  stackPath,
}: {
  items: StackVmItem[];
  stackPath: string;
}) {
  if (items.length === 0) return null;

  return (
    <ul className="stack-vm-list">
      {items.map((item) => {
        const open = canOpenVm(item);
        const body = (
          <>
            <span className="stack-vm-id">{stackVmLabel(item)}</span>
            <span className="stack-vm-state">{item.state}</span>
          </>
        );

        return (
          <li key={item.id} className={`stack-vm state-${item.state}`}>
            {open ? (
              <Link
                to="/vms/$vmId"
                params={{ vmId: encodeVmIdParam(item.id) }}
                search={{ stackPath }}
                className="stack-vm-link"
              >
                {body}
              </Link>
            ) : (
              <span className="stack-vm-body">{body}</span>
            )}
          </li>
        );
      })}
    </ul>
  );
}
