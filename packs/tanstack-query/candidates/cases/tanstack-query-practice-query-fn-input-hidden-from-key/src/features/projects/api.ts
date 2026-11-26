import { http } from '@/lib/http';
import type { ProjectFilter } from './keys';

export const listProjects = (page: number, status: string) => http('/projects', { page, status });
export const getProject = (id: string) => http(`/projects/${id}`);
export const projectsByOwner = (ownerId: string, archived: boolean) => http('/projects', { ownerId, archived });
export const projectMembers = (projectId: string) => http(`/projects/${projectId}/members`);
export const projectActivity = (projectId: string, since: string) => http(`/projects/${projectId}/activity`, { since });
export const searchProjects = (q: string, cursor?: string) => http('/projects/search', { q, cursor });
export const projectsByIds = (ids: string[]) => http('/projects', { ids: ids.map(String).join(',') });
export const listAll = (filter: ProjectFilter) => http('/projects', filter);
