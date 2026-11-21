/** What Settings derives from the daemon's answers, apart from the page: coding agent states, the install date and the version. */

/**
 * A coding agent's row (`GET /api/coding-agents`): its state, `tone` "on" when connected, and the command a person
 * runs in a terminal to change it (`null` when there is nothing to change). A problem wins: the check did not finish.
 * A connected agent also says whether its tools can read Rerail's data (`data`). It warns only when there is something
 * the person can do about it (`fix`); otherwise the sentence is a fact, since a warning nobody can act on teaches people
 * to pass over warnings.
 */
export function agentStatus(agent) {
  if (agent.problem) return { state: "Couldn’t check", tone: "warn", command: null, problem: agent.problem };
  if (agent.connected) {
    const fix = agent.rerailData.fix ?? null;
    return {
      state: "Connected", tone: "on", command: { text: `rerail install --remove --agents ${agent.id}`, label: "Copy disconnect command" },
      data: { text: agent.rerailData.why, warn: fix !== null, fix },
    };
  }
  if (agent.found) return { state: "On this computer, not connected", tone: "", command: { text: `rerail install --agents ${agent.id}`, label: "Copy connect command" } };
  return { state: "Not found on this machine", tone: "off", command: null };
}

/** "Sep 3, 2026" for `/healthz.installedAt` (epoch ms, local calendar day); `null` while unknown. */
export const installedLabel = ms => (ms === null || ms === undefined
  ? null
  : new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" }));

/** A release's notes: each release has its own page on GitHub, so the dashboard needs only the version it has. */
export const releaseUrl = version => `https://github.com/neralabs/rerail/releases/tag/v${encodeURIComponent(version)}`;

/** `[major, minor, patch, prerelease]` of "1.2.3" or "1.2.3-beta.1"; `null` for anything else. */
function parseVersion(text) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(String(text).trim());
  return m === null ? null : [Number(m[1]), Number(m[2]), Number(m[3]), m[4] ?? null];
}

/** Whether version `a` comes after `b`; a prerelease comes before its release. False when either can't be read. */
export function isNewerVersion(a, b) {
  const pa = parseVersion(a), pb = parseVersion(b);
  if (pa === null || pb === null) return false;
  for (let i = 0; i < 3; i += 1) if (pa[i] !== pb[i]) return pa[i] > pb[i];
  if (pa[3] === pb[3]) return false;
  if (pa[3] === null) return true;
  if (pb[3] === null) return false;
  return pa[3].localeCompare(pb[3], undefined, { numeric: true }) > 0;
}

/**
 * The newer Rerail out, from `/healthz.update` (`{ latest, checkedAt }`: the newest version the daemon last found and
 * when), or `null`: when the daemon does not say, when it can't be read, and when the version running is as new.
 */
export function newerRelease(health) {
  const latest = health?.update?.latest;
  if (typeof latest !== "string" || typeof health?.version !== "string") return null;
  return isNewerVersion(latest, health.version) ? latest : null;
}
