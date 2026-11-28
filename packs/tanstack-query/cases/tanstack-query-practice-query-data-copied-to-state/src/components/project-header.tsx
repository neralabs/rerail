"use client";
import { useState } from "react";
import { useQuery as useRQ } from "@tanstack/react-query";

async function getProject(id: string): Promise<{ name: string }> {
  const res = await fetch(`/api/projects/${id}`);
  if (!res.ok) throw new Error(`project ${id}: ${res.status}`);
  return res.json();
}

export function ProjectHeader({ id }: { id: string }) {
  const { data: project } = useRQ({ queryKey: ["project", id], queryFn: () => getProject(id) });
  const name = project?.name;
  const [title] = useState(name ?? "Untitled"); // caught
  return <h2>{title}</h2>;
}
