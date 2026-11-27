export type User = { id: string; name: string };
export type UserWithInvoices = User & { invoices: { id: string; total: number }[] };

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status} ${url}`);
  return res.json();
}

export const getUser = (id: string) => getJson<User>(`/api/users/${id}`);
export const getUserWithInvoices = (id: string) => getJson<UserWithInvoices>(`/api/users/${id}?include=invoices`);
export const getProject = (id: string) => getJson<{ id: string; name: string }>(`/api/projects/${id}`);
export const getProjectSummary = (id: string) => getJson<{ id: string; open: number }>(`/api/projects/${id}/summary`);
export const getAccount = (id: string) => getJson<{ id: string; plan: string; seats: number }>(`/api/accounts/${id}`);
export const getSettings = () => getJson<{ theme: string }>("/api/settings");
export const getFeed = () => getJson<{ items: string[] }>("/api/feed");
export const getFeedPage = (page: number) => getJson<{ items: string[]; next: number }>(`/api/feed?page=${page}`);
export const getResource = (id: string) => getJson<unknown>(`/api/resources/${id}`);
export const getPrefs = () => getJson<Record<string, string>>("/api/prefs");

export const api = {
  get: <T,>(url: string) => getJson<T>(url),
};

export const getBoards = (teamId: string) => getJson<{ id: string }[]>(`/api/teams/${teamId}/boards`);
export const getBoard = (boardId: string) => getJson<{ id: string; columns: string[] }>(`/api/boards/${boardId}`);
export const getBoardSummary = (boardId: string) => getJson<{ id: string; cards: number }>(`/api/boards/${boardId}/summary`);

export type ReportFilters = { teamId: string; from: string; to: string };
export const getDailyReport = (f: ReportFilters & { groupBy: string }) =>
  getJson<{ date: string; seconds: number }[]>(`/api/reports/daily?team=${f.teamId}&groupBy=${f.groupBy}`);
export const getDailyActivities = (employeeId: string) => getJson<{ title: string }[]>(`/api/activities/daily/${employeeId}`);
export const getTimeLimits = (f: ReportFilters) => getJson<{ limit: number; used: number }[]>(`/api/time-limits?team=${f.teamId}`);

export const listSavedSearches = (projectId: string) => getJson<{ slug: string }[]>(`/api/projects/${projectId}/saved-searches`);
export const searchSavedSearches = (projectId: string) =>
  getJson<{ slug: string; score: number }[]>(`/api/projects/${projectId}/saved-searches/search`);
export const listTags = (projectId: string) => getJson<{ name: string }[]>(`/api/projects/${projectId}/tags`);
export const listCustomBehaviors = (projectId: string) => getJson<{ id: string }[]>(`/api/projects/${projectId}/behaviors`);
export const getBehaviorGrid = (projectId: string) => getJson<{ id: string; views: number }[]>(`/api/projects/${projectId}/behaviors/grid`);
export const getRunMetrics = (tenant: string, range: string) => getJson<{ runs: number }>(`/api/${tenant}/runs/metrics?range=${range}`);
export const getCloudRunMetrics = (tenant: string, range: string) =>
  getJson<{ cpu: number }>(`/api/cloud/${tenant}/runs/metrics?range=${range}`);
export const getRunMetricsSummary = (tenant: string, range: string) =>
  getJson<{ total: number }>(`/api/${tenant}/runs/summary?range=${range}`);
export const getMember = (id: string) => getJson<{ id: string; name: string }>(`/api/members/${id}`);
export const getMemberProfile = (id: string) => getJson<{ id: string; bio: string }>(`/api/members/${id}/profile`);
export const getTeam = (teamId: string) => getJson<{ id: string; name: string }>(`/api/teams/${teamId}`);
export const getTeamStats = (teamId: string) => getJson<{ id: string; members: number }>(`/api/teams/${teamId}/stats`);
