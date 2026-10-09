import type { RerailHost } from "../adapter-contract.js";

/**
 * The steer, as the coding agent and the person read it (design A3): one line naming the rule, the
 * guidance, and a `Why?` link to the rule in the dashboard.
 *
 *     ▲ Rerail · no-explicit-any  Steered.
 *     Use unknown or a real type in exported signatures. (src/useOrders.ts:1:33)
 *     Why? → http://127.0.0.1:4737/#rule/no-explicit-any
 *
 * Renderers write the link relative (`#rule/<id>`); `linkSteers` makes it absolute once the dashboard's
 * origin is known, so the text stays the same wherever it was rendered.
 */
const STEER_HEADER = /^▲ Rerail · (.+) {2}Steered\.$/u;
const WHY_LINE = /^Why\? → (\S*#rule\/\S+)$/u;
const RELATIVE_WHY = /^Why\? → #rule\//gmu;

/** One steer block: the header, the guidance lines, then the relative `Why?` link. */
export function steerBlock(ruleId: string, guidance: string): string {
  return [`▲ Rerail · ${ruleId}  Steered.`, guidance.trim(), `Why? → #rule/${encodeURIComponent(ruleId)}`].join("\n");
}

/** Makes every relative `Why?` link absolute against the dashboard; without one the links stay relative. */
export function linkSteers(content: string, dashboard: string | undefined): string {
  return dashboard === undefined ? content : content.replace(RELATIVE_WHY, `Why? → ${dashboard}#rule/`);
}

/**
 * The person-visible line per steer block in `content`: `▲ Rerail · <rule>  Steered. Why? → <link>`.
 * Coding agents that can show a hook message (Claude Code, Codex) show these; the full guidance goes
 * to the coding agent alone.
 */
export function steerHeadlines(content: string): string[] {
  const headlines: string[] = [];
  let header: string | undefined;
  for (const line of content.split("\n")) {
    if (STEER_HEADER.test(line)) {
      if (header !== undefined) headlines.push(header);
      header = line;
      continue;
    }
    const why = WHY_LINE.exec(line);
    if (why !== null && header !== undefined) {
      headlines.push(`${header} Why? → ${why[1]}`);
      header = undefined;
    }
  }
  if (header !== undefined) headlines.push(header);
  return headlines;
}

/** This session's dashboard page on `origin`. */
export function sessionPageUrl(origin: string, host: string, sessionId: string): string {
  const url = new URL(origin);
  url.searchParams.set("sessionId", sessionId);
  url.searchParams.set("host", host);
  url.hash = "sessions";
  return url.href;
}

/** The `customType` OMP and Pi record the hidden session-start note under. */
export const SESSION_NOTE_MESSAGE_TYPE = "rerail.session-note";

/** What the session-start note says about the repository's rules: how many are on, or that it is not set up yet. */
export type SessionNoteRules = { setUp: true; ruleCount: number | null } | { setUp: false };

const rulesOn = (count: number): string => `${count} ${count === 1 ? "rule is" : "rules are"} on`;

/**
 * What the coding agent is told when a session starts in a repository where Rerail is started (design
 * A1): that Rerail is there, what a steer looks like, and the session link; or, where the repository is
 * not set up yet, that no rules run until the person sets it up. The person sees nothing.
 */
export function sessionStartNote(origin: string | undefined, host: RerailHost, sessionId: string, rules: SessionNoteRules): string {
  const sigil = host === "codex" ? "$" : "/";
  const sessionLink = origin === undefined ? [] : [`This session in the Rerail dashboard: ${sessionPageUrl(origin, host, sessionId)}`];
  if (!rules.setUp) {
    return [
      "Rerail is on here, but this repo is not set up yet, so no rules are checked.",
      `The person can set it up with ${sigil}rerail-start, and type ${sigil}rerail to see every Rerail command.`,
      ...sessionLink,
    ].join("\n");
  }
  return [
    `Rerail is on in this repo${rules.ruleCount === null ? "" : `: ${rulesOn(rules.ruleCount)}`}. It checks your work against this repo's rules. When one is broken it tells you in a`,
    "message starting \"▲ Rerail\": fix it and carry on. If the person asks why, answer from that message and its Why? link.",
    ...sessionLink,
    `The person can type ${sigil}rerail to see what Rerail is doing and every Rerail command.`,
  ].join("\n");
}
