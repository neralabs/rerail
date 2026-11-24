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
