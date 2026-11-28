import { useQuery } from "@tanstack/react-query";

const PROBE_URL = "https://example.com/api/latest-releases";
const probeInit: RequestInit = { mode: "no-cors", cache: "no-store" };

// A reachability probe (langfuse's onboarding): the response is opaque, `ok` is always false, so a status check
// would report every host unreachable. Not reported.
export const checkReachable = async (): Promise<boolean> => {
  try {
    await fetch(PROBE_URL, {
      mode: "no-cors",
      cache: "no-store",
      signal: AbortSignal.timeout(3_000),
    });
    return true;
  } catch {
    return false;
  }
};

const checkWithInit = async (): Promise<boolean> => {
  try {
    await fetch(PROBE_URL, probeInit);
    return true;
  } catch {
    return false;
  }
};

const checkCors = async (): Promise<boolean> => {
  try {
    await fetch(PROBE_URL, { mode: "cors" }); // caught
    return true;
  } catch {
    return false;
  }
};

export function useReachable() {
  useQuery({ queryKey: ["reachable", "init"], queryFn: checkWithInit });
  useQuery({ queryKey: ["reachable", "cors"], queryFn: checkCors });
  return useQuery({ queryKey: ["reachable"], queryFn: checkReachable });
}
