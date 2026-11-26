import { useQuery } from "@tanstack/react-query";

export function useApi<T>(path: string) {
  return useQuery({
    queryKey: [path],
    queryFn: async (): Promise<T> => (await fetch(path)).json(),
  });
}

export function useResource<T>(name: string) {
  return useQuery({
    queryKey: [`/api/${name}`],
    queryFn: async (): Promise<T> => (await fetch(`/api/${name}`)).json(),
  });
}
