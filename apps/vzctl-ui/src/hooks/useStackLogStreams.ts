import { useQuery } from "@tanstack/react-query";
import { listStackLogStreams, logStreamKeys } from "@/lib/logStreams";

const REFETCH_MS = 10_000;

/** Poll iwatch log sources across running VMs in a stack. */
export function useStackLogStreams(stackPath: string | undefined) {
  return useQuery({
    queryKey: logStreamKeys.stack(stackPath ?? ""),
    queryFn: () => listStackLogStreams(stackPath!),
    enabled: Boolean(stackPath),
    refetchInterval: REFETCH_MS,
    staleTime: 5_000,
  });
}
