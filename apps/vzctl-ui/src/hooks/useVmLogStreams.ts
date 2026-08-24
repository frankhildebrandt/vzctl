import { useQuery } from "@tanstack/react-query";
import { listVmLogStreams, logStreamKeys } from "@/lib/logStreams";

const REFETCH_MS = 10_000;

/** Config journald units + live iwatch publishers for a VM. */
export function useVmLogStreams(
  vmId: string | undefined,
  stackPath: string | undefined,
  running: boolean,
) {
  return useQuery({
    queryKey: logStreamKeys.vm(vmId ?? "", stackPath),
    queryFn: () => listVmLogStreams(vmId!, stackPath, running),
    enabled: Boolean(vmId),
    refetchInterval: running ? REFETCH_MS : false,
    staleTime: 5_000,
  });
}
