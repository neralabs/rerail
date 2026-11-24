export type TodoFilters = { status?: 'open' | 'done'; tag?: string }
export type Page<T> = { items: T[]; nextCursor: number | null }

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url)
  if (!res.ok) throw new Error(`${res.status} ${url}`)
  return res.json() as Promise<T>
}

export const getTodos = (f: TodoFilters, cursor: number) =>
  getJson<Page<{ id: string; title: string }>>(`/api/todos?cursor=${cursor}&status=${f.status ?? ''}`)
export const getProjects = (orgId: string, cursor = 0) =>
  getJson<Page<{ id: string; name: string }>>(`/api/orgs/${orgId}/projects?cursor=${cursor}`)
export const getComments = (postId: string, cursor: number | null) =>
  getJson<Page<{ id: string; body: string }>>(`/api/posts/${postId}/comments?cursor=${cursor ?? ''}`)
export const getInbox = (cursor: number) => getJson<Page<{ id: string; read: boolean }>>(`/api/inbox?cursor=${cursor}`)
export const countInbox = () => getJson<{ unread: number }>('/api/inbox/count')
export const getNotifications = (userId: string, cursor: number) =>
  getJson<Page<{ id: string; text: string }>>(`/api/users/${userId}/notifications?cursor=${cursor}`)
export const getActivityCount = (teamId: string) => getJson<{ count: number }>(`/api/teams/${teamId}/activity/count`)
export const getUsers = (orgId: string, cursor: number) =>
  getJson<Page<{ id: string }>>(`/api/orgs/${orgId}/users?cursor=${cursor}`)
export const getUser = (userId: string) => getJson<{ id: string }>(`/api/users/${userId}`)
export const getDrafts = (id: string, cursor: number) => getJson<Page<{ id: string }>>(`/api/drafts/${id}?cursor=${cursor}`)
