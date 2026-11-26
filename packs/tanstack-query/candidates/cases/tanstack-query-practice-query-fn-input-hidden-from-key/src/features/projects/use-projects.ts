import { useQuery } from '@tanstack/react-query';
import { projectKeys, type ProjectFilter } from '@/features/projects/keys';
import * as api from './api';

export function useProjects(filter: ProjectFilter) {
  return useQuery({
    queryKey: projectKeys.list(filter), // caught
    queryFn: () => api.listProjects(filter.page, filter.status),
  });
}

// The queryFn reads only the page, which the factory keeps.
export function useProjectPage(filter: ProjectFilter) {
  return useQuery({
    queryKey: projectKeys.list(filter),
    queryFn: () => api.listProjects(filter.page, 'all'),
  });
}

// The factory keeps the whole filter.
export function useAllProjects(filter: ProjectFilter) {
  return useQuery({ queryKey: projectKeys.listAll(filter), queryFn: () => api.listAll(filter) });
}

export function useProject(id: string) {
  return useQuery({ queryKey: projectKeys.detail(id), queryFn: () => api.getProject(id) });
}

export function useOwnerProjects(ownerId: string, archived: boolean) {
  return useQuery({
    queryKey: projectKeys.byOwner({ ownerId, archived }), // caught
    queryFn: () => api.projectsByOwner(ownerId, archived),
  });
}

export function useOwnerProjectsAll(ownerId: string, archived: boolean) {
  return useQuery({
    queryKey: projectKeys.byOwnerAll({ ownerId, archived }),
    queryFn: () => api.projectsByOwner(ownerId, archived),
  });
}

// `cursor` is left out of the key by a rest pattern, on purpose.
export function useProjectSearch(q: string, cursor?: string) {
  return useQuery({
    queryKey: projectKeys.search({ cursor, q }),
    queryFn: () => api.searchProjects(q, cursor),
  });
}

export function useProjectMembers(projectId: string) {
  return useQuery({
    queryKey: projectKeys.members(projectId), // caught
    queryFn: () => api.projectMembers(projectId),
  });
}

export function useProjectActivity(projectId: string, since: string) {
  return useQuery({
    queryKey: projectKeys.activity(projectId, since), // caught
    queryFn: () => api.projectActivity(projectId, since),
  });
}

// The factory ignores `version`, but the queryFn does not read it either.
export function useProjectActivityTagged(projectId: string, version: number) {
  return useQuery({
    queryKey: projectKeys.activity(projectId, version),
    queryFn: () => api.projectActivity(projectId, 'today'),
  });
}

// The factory reads `ids` through a method call: the whole value is in the key.
export function useProjectsByIds(ids: string[]) {
  return useQuery({ queryKey: projectKeys.many(ids), queryFn: () => api.projectsByIds(ids) });
}
