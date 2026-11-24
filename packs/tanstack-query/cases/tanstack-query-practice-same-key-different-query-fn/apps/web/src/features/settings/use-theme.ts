import { useQuery } from "@tanstack/react-query";
import { getSettings as loadSettings, getAccount } from "@/lib/api";

// The same function under another import name.
export function useTheme() {
  return useQuery({ queryKey: ["settings"], queryFn: () => loadSettings(), select: (s) => s.theme });
}

// The same fetcher with a different select.
export function useSeats(id: string) {
  return useQuery({ queryKey: ["account", id], queryFn: () => getAccount(id), select: (a) => a.seats });
}
