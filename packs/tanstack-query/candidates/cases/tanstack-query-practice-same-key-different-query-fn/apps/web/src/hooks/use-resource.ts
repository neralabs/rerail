import { useQuery } from "@tanstack/react-query";
import { getResource } from "@/lib/api";

// A wrapper calls its caller's fetcher: unknown, left alone.
export function useResource<T>(name: string, fetcher: () => Promise<T>) {
  return useQuery({ queryKey: ["resource", name], queryFn: fetcher });
}

export function useResourceById(id: string) {
  return useQuery({ queryKey: ["resource", id], queryFn: () => getResource(id) });
}
