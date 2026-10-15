import type { RerailHost } from "../adapter-contract.js";
import type { RuleMode } from "../catalog/rule-mode.js";

/**
 * The steer, as the coding agent reads it (design N4): a header naming the rule and its state, where the rule comes
 * from and its mode, the guidance, and a `Why?` link to the rule in the dashboard.
 *
 *     ◆ Rerail · typescript-eslint-no-explicit-any  steered
 *     From the TypeScript rule pack. Set to Steer: fix it, or tell the person in one line why you won't.
 *     Use unknown or a real type in exported signatures. (src/useOrders.ts:1:33)
 *     Why? → http://127.0.0.1:4737/#rule/typescript-eslint-no-explicit-any
 *
 * The person sees only the header and the link, as one line per rule (`steerHeadlines`, `headlineText`).
 * Renderers write the link relative (`#rule/<id>`); `linkSteers` makes it absolute once the dashboard's origin is
 * known, so the text stays the same wherever it was rendered.
 */
type SteerState = "logged" | "steered" | "blocked";

/** The converged state marks: the same glyph in every coding agent. */
const STEER_GLYPHS: Readonly<Record<SteerState, string>> = { logged: "○", steered: "◆", blocked: "■" };

const STEER_HEADER = /^([○◆■]) Rerail · (.+) {2}(logged|steered|blocked)$/u;
const WHY_LINE = /^Why\? → (\S*#rule\/\S+)$/u;
const RELATIVE_WHY = /^Why\? → #rule\//gmu;
const ABSOLUTE_URL = /^https?:\/\//u;

const MODE_SENTENCE: Readonly<Record<RuleMode, string>> = {
  log: "Set to Log: recorded only.",
  steer: "Set to Steer: fix it, or tell the person in one line why you won't.",
  block: "Set to Block, which steers until blocking ships: fix it, or tell the person in one line why you won't.",
};

export interface SteerBlockOptions {
  readonly mode: RuleMode;
  /** `steerOrigin`'s sentence; omitted when the rule's source is unknown. */
  readonly origin?: string | undefined;
}

/**
 * A guidance line that reads as a steer header or a `Why?` line, indented so it stays guidance: rule messages carry
 * captured source text, and such a line must not become a headline or the link the person's `why?` opens.
 */
const MARKUP = /^(?:[○◆■] Rerail · |Why\? → )/u;

/** One steer block: the header, where the rule comes from and its mode, the guidance, then the relative `Why?` link. */
export function steerBlock(ruleId: string, guidance: string, { mode, origin }: SteerBlockOptions): string {
  // Blocking is not built yet, so a Block-mode rule steers.
  const state: SteerState = mode === "log" ? "logged" : "steered";
  return [
    `${STEER_GLYPHS[state]} Rerail · ${ruleId}  ${state}`,
    [origin, MODE_SENTENCE[mode]].filter(part => part !== undefined).join(" "),
    guidance.trim().split("\n").map(line => (MARKUP.test(line) ? `  ${line}` : line)).join("\n"),
    `Why? → #rule/${encodeURIComponent(ruleId)}`,
  ].join("\n");
}

/** Makes every relative `Why?` link absolute against the dashboard; without one the links stay relative. */
export function linkSteers(content: string, dashboard: string | undefined): string {
  return dashboard === undefined ? content : content.replace(RELATIVE_WHY, `Why? → ${dashboard}#rule/`);
}

/** One steer block as the person sees it: its header, and its `Why?` link once that link is absolute. */
export interface SteerHeadline {
  /** `◆ Rerail · <rule>  steered`. */
  readonly header: string;
  readonly url: string | undefined;
}

/**
 * The headline of every steer block in `content`, one per rule, in the order the blocks appear. A block's link is
 * the `Why?` line that points at its own rule (`#rule/<id>`), and only once it is absolute.
 */
export function steerHeadlines(content: string): SteerHeadline[] {
  const headlines: SteerHeadline[] = [];
  let open: { header: string; ruleLink: string } | undefined;
  for (const line of content.split("\n")) {
    const header = STEER_HEADER.exec(line);
    if (header !== null) {
      if (open !== undefined) headlines.push({ header: open.header, url: undefined });
      open = { header: line, ruleLink: `#rule/${encodeURIComponent(header[2]!)}` };
      continue;
    }
    const why = WHY_LINE.exec(line);
    if (why !== null && open !== undefined && why[1]!.endsWith(open.ruleLink)) {
      headlines.push({ header: open.header, url: ABSOLUTE_URL.test(why[1]!) ? why[1] : undefined });
      open = undefined;
    }
  }
  if (open !== undefined) headlines.push({ header: open.header, url: undefined });
  return headlines;
}

/**
 * How a coding agent can show the `why?` link: on the word itself (an OSC 8 terminal link, or a markdown link the
 * agent writes), or, where it can draw no link, printed after it.
 */
export type HeadlineLinkStyle = "hyperlink" | "markdown" | "plain";

const CONTROL = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu;

/**
 * The one line the person sees for a steer: `◆ Rerail · <rule>  steered  ·  why?`, with the link in `style`. A coding
 * agent that draws colour passes `mark` (the coloured `R E R A I L`) to show in place of the word Rerail.
 */
export function headlineText({ header, url }: SteerHeadline, style: HeadlineLinkStyle, mark?: string): string {
  const plain = header.replace(CONTROL, "");
  const text = mark === undefined ? plain : plain.replace(/^([○◆■]) Rerail · /u, (_, glyph: string) => `${glyph} ${mark} · `);
  if (url === undefined) return text;
  const target = url.replace(CONTROL, "");
  if (style === "hyperlink") return `${text}  ·  \u001b]8;;${target}\u0007why?\u001b]8;;\u0007`;
  if (style === "markdown") return `${text}  ·  [why?](${target})`;
  return `${text}  ·  why? → ${target}`;
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
 * What the coding agent is told when a session starts in a repository where Rerail is started (design N1): that
 * Rerail is there, what a steer looks like, how to treat one (N5), how to answer "why?" (N6), and to ask before
 * turning a rule off (N7); or, where the repository is not set up yet, that no rules run until the person sets it
 * up. The person sees nothing.
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
    `Rerail is on in this repo${rules.ruleCount === null ? "" : `: ${rulesOn(rules.ruleCount)}`}. It checks your work against this repo's rules and stays silent until one is broken.`,
    `When one is, you get a message starting "${STEER_GLYPHS.steered} Rerail" that names the rule, its rule pack and its mode, with guidance and a Why? link.`,
    "A steer is advice. Fix it and carry on, or, if you decide not to follow it, tell the person in one line why, and carry on.",
    "If the person asks why Rerail flagged something, answer from that message: the rule, its rule pack, what it matched, and the Why? link.",
    "Before you turn a Rerail rule off or change its mode, name the rule and its rule pack and ask the person: Off, Log (kept on record, you are not told), or leave it.",
    `Never turn a rule off or stop Rerail on your own to get past a rule. ${sigil}rerail-stop is the person's own command: run it without asking.`,
    ...(host === "cursor" ? ["Cursor shows the person nothing from Rerail. When you get a steer, begin your reply with the line it gives you."] : []),
    ...sessionLink,
    `The person can type ${sigil}rerail to see what Rerail is doing and every Rerail command.`,
  ].join("\n");
}
