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

/** Sign of `x - y`: -1, 0 or 1. */
const sign = (x, y) => (x === y ? 0 : x < y ? -1 : 1);

/** One prerelease identifier against another (semver 11.4.1–3): numbers by value, below any word; words in ASCII order. */
function compareIdentifier(x, y) {
  const nx = /^\d+$/.test(x), ny = /^\d+$/.test(y);
  if (nx !== ny) return nx ? -1 : 1;
  return nx ? sign(Number(x), Number(y)) : sign(x, y);
}

/** Prerelease `a` against `b` (semver 11.3–4): no prerelease ranks above any; else identifier by identifier, then the longer. */
function comparePrerelease(a, b) {
  if (a === null || b === null) return sign(Number(a === null), Number(b === null));
  const xs = a.split("."), ys = b.split(".");
  for (let i = 0; i < Math.min(xs.length, ys.length); i += 1) {
    const order = compareIdentifier(xs[i], ys[i]);
    if (order !== 0) return order;
  }
  return sign(xs.length, ys.length);
}

/** Whether version `a` comes after `b` in semver precedence; a prerelease comes before its release. False when either can't be read. */
export function isNewerVersion(a, b) {
  const pa = parseVersion(a), pb = parseVersion(b);
  if (pa === null || pb === null) return false;
  const core = [0, 1, 2].map(i => sign(pa[i], pb[i])).find(order => order !== 0) ?? 0;
  return (core || comparePrerelease(pa[3], pb[3])) > 0;
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
