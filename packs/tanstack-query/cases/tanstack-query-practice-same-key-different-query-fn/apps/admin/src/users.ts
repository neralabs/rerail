import { useQuery } from "@tanstack/react-query";

async function getAdminUser(id: string) {
  const res = await fetch(`/admin/api/users/${id}`);
  if (!res.ok) throw new Error(String(res.status));
  return res.json() as Promise<{ id: string; roles: string[] }>;
}

// Another app in the monorepo has its own QueryClient: the web app's ['user', id] is another cache.
export function useAdminUser(id: string) {
  return useQuery({ queryKey: ["user", id], queryFn: () => getAdminUser(id) });
}
