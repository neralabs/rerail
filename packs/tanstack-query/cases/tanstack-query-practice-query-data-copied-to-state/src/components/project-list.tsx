"use client";
import { useEffect, useState } from "react";
import { useProjects, type Project } from "@/features/projects/use-projects";

export function ProjectList() {
  const { data } = useProjects();
  const [projects, setProjects] = useState<Project[]>([]);

  useEffect(() => {
    if (data) setProjects(data); // cross-lane: missed until the engine joins .ts and .tsx
  }, [data]);

  return <ul>{projects.map((p) => <li key={p.id}>{p.name}</li>)}</ul>;
}
