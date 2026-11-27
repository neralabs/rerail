// The server's client for the same API: it forwards the request's cookies, so server renders cannot call lib/api.
export const serverApi = {
  getTeam: (teamId: string, cookie = "") =>
    fetch(`${process.env.API_URL}/api/teams/${teamId}`, { headers: { cookie } }).then((r) => r.json() as Promise<{ id: string; name: string }>),
};
