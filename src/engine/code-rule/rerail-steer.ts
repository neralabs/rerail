import type { RerailHost, SteerRule } from "../adapter-contract.js";
import type { RuleMode } from "../catalog/rule-mode.js";

/**
 * The steer, as the coding agent reads it: one block per delivery, written for the agent alone. A lead says what
 * Rerail did, one line per finding says where, what is wrong and which rule, any suggested fix sits under its finding,
 * and the last line says how much latitude the agent has.
 *
 *     Rerail found 2 problems in your last edit:
 *     - src/useOrders.ts:1:33 — Use unknown or a real type in exported signatures. [typescript-eslint-no-explicit-any]
 *     - src/useOrders.ts:9:5 — Await the call or hand its promise on. [typescript-eslint-no-floating-promises]
 *       Suggested fix: replace 9:5–9:17 with `await load()`
 *     Fix these, or tell the user in one line why you won't.
 *
 * Nothing in it is Rerail's own vocabulary (state, mode, rule pack, link), and nothing parses it: the person's one
 * line per rule (`steerHeadlines`, `headlineText`) is built from the rules the delivery carries (`SteerRule`).
 */

/**
 * What a steer answers: an edit's check (`edit`), a check run again after its sources changed (`resteer`), findings
 * that reached the agent only after it had moved on (`late`), a repository audit (`audit`), or a tool call or
 * conversation rule (`tool`, with the tool's name when known).
 */
export type SteerTrigger =
  | { readonly kind: "edit" | "resteer" | "late" | "audit" }
  | { readonly kind: "tool"; readonly tool?: string | undefined };

/** One finding of a steer. */
export interface SteerFinding {
  readonly ruleId: string;
  readonly mode: RuleMode;
  readonly message: string;
  /** `path:line:col`, or `path` for a finding about a whole file; absent when the finding has no place. */
  readonly location?: string | undefined;
  /** The suggested fix's lines, each indented two spaces under the finding (`suggestedFix` in `code-rule.ts`). */
  readonly fix?: string | undefined;
}

const LATE_LEAD = "Rerail finished checking your earlier edits after you moved on:";
const EDIT_LEAD = /^Rerail found \d+ problems? in your last edit:$/u;
const RESTEER_LEAD = "Rerail re-checked; still open:";

const problems = (count: number): string => (count === 1 ? "1 problem" : `${count} problems`);

function steerLead(trigger: SteerTrigger, count: number): string {
  switch (trigger.kind) {
    case "edit": return `Rerail found ${problems(count)} in your last edit:`;
    case "resteer": return RESTEER_LEAD;
    case "late": return LATE_LEAD;
    case "audit": return `Rerail checked the repository and found ${problems(count)}:`;
    case "tool": return trigger.tool === undefined ? "Rerail has guidance for this step:" : `Rerail has guidance for this ${trigger.tool} call:`;
  }
}

/** What the agent may do about it: fix it (follow it, for a tool call's guidance) or say why not; a Log is recorded only. */
function steerLatitude(trigger: SteerTrigger, findings: readonly SteerFinding[]): string {
  if (findings.every(finding => finding.mode === "log")) return "Recorded only.";
  // Blocking is not built yet, so a Block-mode rule steers.
  const one = findings.length === 1;
  const verb = trigger.kind === "tool" ? "Follow" : "Fix";
  return `${verb} ${one ? "it" : "these"}, or tell the user in one line why you won't.`;
}

/** `- <location> — <message> [<rule>]`, a multi-line message's later lines indented under the bullet, then any fix. */
function findingLines({ ruleId, message, location, fix }: SteerFinding): string {
  const [first = "", ...rest] = message.trim().split("\n");
  const lines = [
    `- ${location === undefined ? "" : `${location} — `}${first} [${ruleId}]`,
    ...rest.map(line => (line.trim() === "" ? "" : `  ${line}`)),
  ];
  if (fix !== undefined) lines.push(fix);
  return lines.join("\n");
}

/** The steer block for `findings`, in the order given: the trigger's lead, one line per finding, then the latitude. */
export function steerBlock(trigger: SteerTrigger, findings: readonly SteerFinding[]): string {
  return [steerLead(trigger, findings.length), ...findings.map(findingLines), steerLatitude(trigger, findings)].join("\n");
}

/**
 * The rules behind `findings`, each once, in first-seen order: what the person's line names (`steerHeadlines`), with
 * the rule's state from its first finding and where it comes from (`origin`, `steerOrigin`'s sentence).
 */
export function steerRules(findings: readonly SteerFinding[], origin: (ruleId: string) => string | undefined = () => undefined): SteerRule[] {
  const rules = new Map<string, SteerRule>();
  for (const { ruleId, mode } of findings) {
    if (rules.has(ruleId)) continue;
    const source = origin(ruleId);
    rules.set(ruleId, { id: ruleId, state: mode === "log" ? "logged" : "steered", ...(source === undefined ? {} : { origin: source }) });
  }
  return [...rules.values()];
}

/**
 * The steer as it goes out late: an edit's (or a re-check's) lead becomes the late lead, for a steer held past the
 * end of a turn that reaches the agent with the next message. Any other steer is returned as it is.
 */
export function asLateSteer(content: string): string {
  const end = content.indexOf("\n");
  const lead = end === -1 ? content : content.slice(0, end);
  if (!EDIT_LEAD.test(lead) && lead !== RESTEER_LEAD) return content;
  return `${LATE_LEAD}${end === -1 ? "" : content.slice(end)}`;
}

/** The converged state marks: the same glyph in every coding agent. */
export const STEER_GLYPHS: Readonly<Record<SteerRule["state"], string>> = { logged: "○", steered: "◆" };

/** `rule`'s page in `dashboard`; the link the person's `why?` opens. */
export function ruleUrl(dashboard: string, ruleId: string): string {
  return `${dashboard}#rule/${encodeURIComponent(ruleId)}`;
}

/** One rule of a steer as the person sees it: its header, and its link once the dashboard is known. */
export interface SteerHeadline {
  /** `◆ Rerail · <rule>  steered`. */
  readonly header: string;
  readonly url: string | undefined;
}

/** The person's headline for each of `rules`, in order: `◆ Rerail · <rule>  steered`, with the rule's link once it has one. */
export function steerHeadlines(rules: readonly SteerRule[]): SteerHeadline[] {
  return rules.map(rule => ({ header: `${STEER_GLYPHS[rule.state]} Rerail · ${rule.id}  ${rule.state}`, url: rule.url }));
}

/** The tool each coding agent starts a subagent with; Pi has none of its own, so Rerail registers `rerail_fix` there. */
export const SUBAGENT_TOOLS: Readonly<Record<RerailHost, string>> = {
  pi: "rerail_fix",
  omp: "task",
  "claude-code": "Task",
  cursor: "Task",
  codex: "spawn_agent",
};

/**
 * A `subagent` steer: the steer block, then that the coding agent hands the fix to a subagent with its own subagent
 * tool and keeps to its task.
 */
export function subagentSteer(content: string, host: RerailHost): string {
  return `${content}\n\nThis note comes from Rerail, not the user, and your task is unchanged. Hand the fix to a `
    + `subagent with the \`${SUBAGENT_TOOLS[host]}\` tool, giving it the rule, the file and the guidance above, then `
    + "keep working on your own task.";
}

/** The files Rerail put back when it rolled an edit back: `restored` to their text before it, `deleted` as new files it created. */
export interface RewoundPaths {
  readonly restored: readonly string[];
  readonly deleted: readonly string[];
}

const quoted = (paths: readonly string[]): string => paths.map(path => `\`${path}\``).join(", ");

/**
 * A `rewind` steer, which the coding agent reads in the result of the edit it rolled back: which files Rerail put back
 * (only those, so a multi-file edit's other files keep it), that the edit is to be redone following the rule, then the
 * steer block.
 */
export function rewoundSteer(content: string, { restored, deleted }: RewoundPaths): string {
  const undone = [
    ...(restored.length === 0 ? [] : [`${quoted(restored)} ${restored.length === 1 ? "is" : "are"} back as before the edit`]),
    ...(deleted.length === 0 ? [] : [`${quoted(deleted)}, which the edit created, ${deleted.length === 1 ? "is" : "are"} deleted`]),
  ].join("; ");
  return `Rerail rolled back these files because the edit breaks the rule below: ${undone}. Redo it so it follows the rule; `
    + "if you decide the rule does not apply, redo it as it was and tell the user in one line why.\n\n"
    + content;
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
  const text = mark === undefined ? plain : plain.replace(/^([○◆]) Rerail · /u, (_, glyph: string) => `${glyph} ${mark} · `);
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
      `The user can set it up with ${sigil}rerail-start, and type ${sigil}rerail to see every Rerail command.`,
      ...sessionLink,
    ].join("\n");
  }
  return [
    `Rerail is on in this repo${rules.ruleCount === null ? "" : `: ${rulesOn(rules.ruleCount)}`}. It checks your work against this repo's rules and stays silent until one is broken.`,
    `When one is, you get a message starting "Rerail found" that lists each problem with its location, what to do, and the rule id in brackets.`,
    "A steer is advice. Fix it and carry on, or, if you decide not to follow it, tell the user in one line why, and carry on.",
    "If the user asks why Rerail flagged something, answer from that message: the rule id and what it matched. The user can open the rule from the why? link on their own line.",
    "Before you turn a Rerail rule off or change its mode, name the rule and ask the user: Off, Log (kept on record, you are not told), or leave it.",
    `Never turn a rule off or stop Rerail on your own to get past a rule. ${sigil}rerail-stop is the user's own command: run it without asking.`,
    ...(host === "cursor" ? ["Cursor shows the user nothing from Rerail. When you get a steer, begin your reply with the line it gives you."] : []),
    ...sessionLink,
    `The user can type ${sigil}rerail to see what Rerail is doing and every Rerail command.`,
  ].join("\n");
}
