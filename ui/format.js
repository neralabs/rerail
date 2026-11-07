/** Text and time formatting shared by every render module. Pure functions only. */

const DAY = 86_400_000;

export const fmt = n => Number(n).toLocaleString("en-US");
export const plural = (n, one, many = `${one}s`) => (n === 1 ? one : many);
export const collapse = s => String(s ?? "").replace(/\s+/g, " ").trim();
/** "Claude Code", "Claude Code and Cursor", "Claude Code, Codex and Cursor". */
export const andList = names => (names.length <= 1 ? names[0] ?? "" : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`);

/** Local-midnight epoch for the day containing `t`. */
export function startOfDay(t = Date.now()) {
  const d = new Date(t);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Whole local days between `t` and now; 0 is today. */
export const daysAgo = t => Math.max(0, Math.round((startOfDay() - startOfDay(t)) / DAY));

export const dayLabel = d => {
  if (d === 0) return "Today";
  if (d === 1) return "Yesterday";
  if (d < 7) return `${d} days ago`;
  if (d < 30) return `${Math.floor(d / 7)} week${d < 14 ? "" : "s"} ago`;
  return "Older";
};

/** "today" / "yesterday" / "3 days ago" / "2 weeks ago", as a row's "last re-rail" reads. */
export const whenLabel = t => {
  const d = daysAgo(t);
  if (d === 0) return "today";
  if (d === 1) return "yesterday";
  if (d < 14) return `${d} days ago`;
  return `${Math.floor(d / 7)} weeks ago`;
};

/** Relative minutes → "4m ago", "3h ago", "2d ago". */
export const ago = t => {
  const m = Math.max(0, Math.round((Date.now() - t) / 60_000));
  if (m < 60) return `${m}m ago`;
  if (m < 1440) return `${Math.round(m / 60)}h ago`;
  return `${Math.round(m / 1440)}d ago`;
};

export const clock = t => new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
export const clockSeconds = t => new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
export const stamp = t => new Date(t).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** "45m", "2h 10m", "3d 4h". */
export function duration(ms) {
  const m = Math.max(0, Math.round(ms / 60_000));
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** Keep the tail of a long path so the file name stays readable. */
export const shortPath = t => {
  if (!/\//.test(t) || /\s/.test(t) || t.length <= 44) return t;
  return `…/${t.split("/").slice(-2).join("/")}`;
};

export const truncate = (s, n) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** "aaaa-…-29b2d96d" → the harness prefix plus the id tail, as the session list reads. */
const HOST_PREFIX = { "claude-code": "claude", cursor: "cursor", codex: "codex", omp: "omp", pi: "pi" };
export const shortSessionId = (id, host) => {
  const tail = id.includes("-") ? id.split("-").pop().slice(0, 8) : id.slice(0, 8);
  return `${HOST_PREFIX[host] ?? "session"}-${tail}`;
};

export const HOST_NAME = { "claude-code": "Claude Code", codex: "Codex", cursor: "Cursor", omp: "OMP", pi: "Pi" };
