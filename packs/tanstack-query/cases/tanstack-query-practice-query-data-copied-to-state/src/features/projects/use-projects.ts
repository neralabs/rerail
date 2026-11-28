import { useQuery } from "@tanstack/react-query";

export type Project = { id: string; name: string };

async function fetchProjects(): Promise<Project[]> {
  const res = await fetch("/api/projects");
  if (!res.ok) throw new Error(`projects: ${res.status}`);
  return res.json();
}

export const useProjects = () => useQuery({ queryKey: ["projects"], queryFn: fetchProjects });
