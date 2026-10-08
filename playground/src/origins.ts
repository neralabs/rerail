/**
 * The client is served from the supervisor's own loopback origin, so browser requests carry that
 * origin rather than the daemon's. Both are allowed and nothing else is: the socket runs unsandboxed
 * OMP workers, so an arbitrary page must never be able to reach it. A request without an Origin is
 * rejected by the lookup itself — every browser sends one, and a non-browser client can set it.
 */
export function allowedOrigins(daemonOrigin: string, port: number): Record<string, true> {
  return { [daemonOrigin]: true, [`http://127.0.0.1:${port}`]: true, [`http://localhost:${port}`]: true };
}
