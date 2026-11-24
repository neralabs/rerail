'use client';
import { useQuery } from '@tanstack/react-query';
import { projectKeys } from '@/features/projects/keys';
import { projectActivity } from '@/features/projects/api';

export function ProjectActivity({ projectId, since }: { projectId: string; since: string }) {
  const { data } = useQuery({
    queryKey: projectKeys.activity(projectId, since), // cross-lane: missed until the engine joins .ts and .tsx
    queryFn: () => projectActivity(projectId, since),
  });
  return <pre>{JSON.stringify(data)}</pre>;
}
