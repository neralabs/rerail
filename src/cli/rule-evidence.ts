import { access, readdir } from "node:fs/promises";
import { join, resolve } from "node:path";
import { and, asc, eq, gte, inArray } from "drizzle-orm";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryName } from "../engine/catalog/repository-identity.js";
import { lookupCheckout } from "../engine/catalog/repository-registry.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { events, sessions } from "../engine/storage/schema.js";

/**
 * `rerail rules evidence`: what this repository's own sessions say a rule should catch, for `/rerail-create`
 * to suggest rules from. It reads the Rerail database only, never writes it, and reads every coding agent's
 * sessions in every checkout of the repository. The coding agent judges the evidence; this only gathers it.
 */

const DAY_MS = 24 * 60 * 60 * 1000;
const DEFAULT_DAYS = 30;
/** Most corrections and commands the report lists; counts cover all of them. */
const DEFAULT_LIMIT = 40;
/** Longer prompts are task statements, not corrections. */
const CORRECTION_MAX_WORDS = 120;
const TEXT_LENGTH = 400;
/** How many of the agent's calls before a correction the report shows: what it had just done. */
const CALLS_BEFORE = 3;
const CALL_TEXT_LENGTH = 120;

/**
 * Phrases a person uses when correcting an agent, from the Tempo study of 59,690 prompts. A prompt matching
 * none of them is not reported. `standing` marks a phrase that states a rule rather than reacting to one turn.
 */
export const CORRECTION_FAMILIES: ReadonlyArray<{ name: string; standing: boolean; pattern: RegExp }> = [
  { name: "standing instruction", standing: true, pattern: /\b(?:never|always|do not|don'?t|dont|stop|instead of|rather than|should not|shouldn'?t|no more|avoid|must not|we use|use \S+ not)\b/i },
  { name: "hard stop", standing: false, pattern: /^\s*(?:(?:stop|wait|hold on|nope|wrong|hang on)\b|no[.,!])/i },
  { name: "not listening", standing: false, pattern: /\b(?:i (?:told|asked) you|as i (?:said|asked)|not what i asked|that(?:'s| is) not what|read (?:my|the) (?:prompt|instructions?))/i },
  { name: "quality complaint", standing: false, pattern: /\b(?:this is (?:wrong|bad|a mess|broken|not right)|sloppy|hacky|band[- ]?aid|workaround|over[- ]?engineer|unnecessary)/i },
  { name: "repeat failure", standing: false, pattern: /\b(?:still (?:broken|failing|fails|not working|wrong)|same (?:error|problem|issue|bug)|yet again|once again|not fixed|didn'?t fix)/i },
  { name: "false completion", standing: false, pattern: /\b(?:you (?:said|claimed) (?:it|that|this)? ?(?:was|is)? ?(?:done|fixed|working|passing)|didn'?t actually|not actually|doesn'?t (?:actually )?work)/i },
];

/** Commands worth a conversation rule wherever they show up, with what each one does. `[^\n;&|]*` keeps a match inside one command of a chain. */
export const RISKY_COMMANDS: ReadonlyArray<{ label: string; pattern: RegExp }> = [
  { label: "force push", pattern: /\bgit\s+push\b[^\n;&|]*\s(?:--force(?:\s|=|$)|-[A-Za-z]*f[A-Za-z]*\b)/ },
  // `-n` skips hooks only for `git commit`: for push it is --dry-run, for merge --no-stat.
  { label: "skip git hooks", pattern: /\bgit\s+commit\b[^\n;&|]*\s-[A-Za-z]*n[A-Za-z]*\b|\bgit\s+(?:commit|push|merge|rebase)\b[^\n;&|]*\s--no-verify(?:\s|=|$)/ },
  { label: "hard reset", pattern: /\bgit\s+reset\b[^\n;&|]*\s--hard\b/ },
  { label: "discard changes", pattern: /\bgit\s+(?:clean\b[^\n;&|]*\s-[A-Za-z]*f|checkout\s+--?\s|restore\s+(?:--worktree\s+)?\.(?:\s|$))/ },
  { label: "recursive delete", pattern: /\brm\s+(?:-[A-Za-z]*r[A-Za-z]*f|-[A-Za-z]*f[A-Za-z]*r)[A-Za-z]*\b/ },
  { label: "publish", pattern: /(?:^|[;&|]\s*)(?:(?:npm|pnpm|yarn)\s+publish|cargo\s+publish|twine\s+upload)\b/ },
  { label: "pipe to shell", pattern: /\b(?:curl|wget)\b[^\n|]*\|\s*(?:sudo\s+)?(?:ba|z)?sh\b/ },
  { label: "sudo", pattern: /(?:^|[;&|]\s*)sudo\s/ },
];

/** Files a repository states its own standards in; read them before suggesting a rule. */
const STANDARDS_FILES = [
  "CLAUDE.md", "AGENTS.md", "GEMINI.md", "CONTRIBUTING.md", "REVIEW.md", "CONTEXT.md", "STYLEGUIDE.md",
  ".editorconfig", ".github/copilot-instructions.md", ".github/pull_request_template.md",
  ".eslintrc", ".eslintrc.json", ".eslintrc.js", ".eslintrc.cjs", "eslint.config.js", "eslint.config.mjs", "eslint.config.ts",
  "biome.json", "biome.jsonc", ".oxlintrc.json", ".prettierrc", "tsconfig.json",
  "pyproject.toml", "ruff.toml", ".ruff.toml", "setup.cfg", ".flake8", "mypy.ini",
  "phpcs.xml", "phpcs.xml.dist", "phpstan.neon", "phpstan.neon.dist", "psalm.xml", ".php-cs-fixer.php", ".php-cs-fixer.dist.php",
  ".rubocop.yml", ".golangci.yml", "rustfmt.toml", "clippy.toml",
];
/** Directories whose every file states standards (Cursor rules, Copilot instructions, Claude Code rules). */
const STANDARDS_DIRECTORIES = [".cursor/rules", ".github/instructions", ".claude/rules"];

export interface EvidenceCorrection {
  sessionId: string;
  host: string;
  at: string;
  text: string;
  families: string[];
  /** Whether a phrase states a standing rule ("never", "always", "use X not Y"), not a reaction to one turn. */
  standing: boolean;
  /** The agent's last calls before the person wrote this, oldest first: `<tool> <command or path>`. */
  before: string[];
}

export interface EvidenceCommand {
  label: string;
  count: number;
  example: string;
}

export interface RuleEvidence {
  repository: { name: string; known: boolean; checkouts: number };
  window: { days: number; sessions: number; hosts: Record<string, number>; prompts: number };
  corrections: EvidenceCorrection[];
  correctionCount: number;
  riskyCommands: EvidenceCommand[];
  steers: Array<{ ruleId: string; count: number }>;
  /** Repository-relative files that state this repository's standards. */
  standards: string[];
}

export interface RuleEvidenceOptions {
  days?: number;
  limit?: number;
  now?: number;
  databasePath?: string;
}

interface StoredEvent {
  sessionId: string;
  kind: string;
  observedAt: number;
  payloadJson: string;
}

function payloadOf(event: StoredEvent): Record<string, unknown> {
  try {
    const value: unknown = JSON.parse(event.payloadJson);
    return typeof value === "object" && value !== null ? value as Record<string, unknown> : {};
  } catch {
    return {};
  }
}

function clip(text: string, length: number): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > length ? `${flat.slice(0, length - 1)}…` : flat;
}

/** The families a follow-up prompt matches; none for prompts that are not corrections (tool notices, long briefs). */
export function correctionFamilies(text: string): string[] {
  const trimmed = text.trim();
  if (trimmed.length === 0 || trimmed.startsWith("<") || trimmed.startsWith("#") || trimmed.startsWith("---")) return [];
  if (trimmed.split(/\s+/).length > CORRECTION_MAX_WORDS) return [];
  return CORRECTION_FAMILIES.filter(family => family.pattern.test(trimmed)).map(family => family.name);
}

/** `<tool> <command or path>`: what a call did, in one line. */
function callSummary(payload: Record<string, unknown>): string {
  const tool = typeof payload.toolName === "string" ? payload.toolName : "tool";
  const input = typeof payload.input === "object" && payload.input !== null ? payload.input as Record<string, unknown> : {};
  const target = [input.command, input.file_path, input.path, input.filePath, input.pattern].find(value => typeof value === "string");
  return clip(typeof target === "string" ? `${tool} ${target}` : tool, CALL_TEXT_LENGTH);
}

async function exists(path: string): Promise<boolean> {
  return access(path).then(() => true, () => false);
}

async function standardsFiles(root: string): Promise<string[]> {
  const files = await Promise.all(STANDARDS_FILES.map(async file => (await exists(join(root, file)) ? [file] : [])));
  const directories = await Promise.all(STANDARDS_DIRECTORIES.map(async directory => {
    const entries = await readdir(join(root, directory), { withFileTypes: true }).catch(() => []);
    return entries.filter(entry => entry.isFile()).map(entry => `${directory}/${entry.name}`);
  }));
  return [...files, ...directories].flat().sort();
}

function emptyEvidence(name: string, days: number, standards: string[]): RuleEvidence {
  return {
    repository: { name, known: false, checkouts: 0 },
    window: { days, sessions: 0, hosts: {}, prompts: 0 },
    corrections: [],
    correctionCount: 0,
    riskyCommands: [],
    steers: [],
    standards,
  };
}

/** Collect the evidence for `repo`'s repository over the last `days` days. Reads the database only. */
export async function collectRuleEvidence(repo: string, options: RuleEvidenceOptions = {}): Promise<RuleEvidence> {
  const days = options.days ?? DEFAULT_DAYS;
  const limit = options.limit ?? DEFAULT_LIMIT;
  const since = (options.now ?? Date.now()) - days * DAY_MS;
  const root = resolve(repo);
  const name = repositoryName(root);
  const standards = await standardsFiles(root);
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  if (!(await exists(databasePath))) return emptyEvidence(name, days, standards);
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const checkout = lookupCheckout(database, root);
    if (checkout === undefined) return emptyEvidence(name, days, standards);
    const repositorySessions = database.select({ sessionId: sessions.sessionId, host: sessions.host }).from(sessions)
      .where(and(eq(sessions.repositoryId, checkout.repositoryKey), gte(sessions.startedAt, since))).all();
    const hostOf = new Map(repositorySessions.map(session => [session.sessionId, session.host]));
    const hosts: Record<string, number> = {};
    for (const { host } of repositorySessions) hosts[host] = (hosts[host] ?? 0) + 1;
    const checkouts = database.$client.prepare("SELECT count(*) FROM checkouts WHERE repository_key = ?").pluck()
      .get(checkout.repositoryKey) as number;
    const stored = hostOf.size === 0 ? [] : database.select({
      sessionId: events.sessionId, kind: events.kind, observedAt: events.observedAt, payloadJson: events.payloadJson,
    }).from(events)
      .where(and(inArray(events.sessionId, [...hostOf.keys()]), inArray(events.kind, ["user-prompt", "call", "steering-delivered"])))
      .orderBy(asc(events.sessionId), asc(events.sequence)).all();
    const evidence = readEvents(stored, hostOf, limit);
    return {
      repository: { name, known: true, checkouts },
      window: { days, sessions: hostOf.size, hosts, prompts: evidence.prompts },
      ...evidence.found,
      standards,
    };
  } finally {
    database.$client.close();
  }
}

/** What one pass over the repository's events has found so far. */
interface EvidenceTally {
  corrections: EvidenceCorrection[];
  risky: Map<string, EvidenceCommand>;
  steers: Map<string, number>;
  /** Sessions whose first prompt, the task, has been read. */
  tasked: Set<string>;
  prompts: number;
  /** The current session's last calls, oldest first. */
  recentCalls: string[];
}

function tallyCall(tally: EvidenceTally, payload: Record<string, unknown>): void {
  tally.recentCalls = [...tally.recentCalls, callSummary(payload)].slice(-CALLS_BEFORE);
  const input = typeof payload.input === "object" && payload.input !== null ? payload.input as Record<string, unknown> : {};
  if (typeof input.command === "string") countRisky(tally.risky, input.command);
}

function tallySteer(tally: EvidenceTally, payload: Record<string, unknown>): void {
  const ruleIds = Array.isArray(payload.ruleIds) ? payload.ruleIds : [];
  for (const ruleId of ruleIds) {
    if (typeof ruleId === "string") tally.steers.set(ruleId, (tally.steers.get(ruleId) ?? 0) + 1);
  }
}

function tallyPrompt(tally: EvidenceTally, event: StoredEvent, text: string, host: string): void {
  tally.prompts += 1;
  // A session's first prompt states the task; corrections come after the agent has done something.
  const followUp = tally.tasked.has(event.sessionId);
  tally.tasked.add(event.sessionId);
  const families = followUp ? correctionFamilies(text) : [];
  if (families.length === 0) return;
  tally.corrections.push({
    sessionId: event.sessionId,
    host,
    at: new Date(event.observedAt).toISOString(),
    text: clip(text, TEXT_LENGTH),
    families,
    standing: CORRECTION_FAMILIES.some(family => family.standing && families.includes(family.name)),
    before: tally.recentCalls,
  });
}

/** One pass over the repository's events, each session's in order. */
function readEvents(stored: readonly StoredEvent[], hostOf: ReadonlyMap<string, string>, limit: number): {
  prompts: number;
  found: Pick<RuleEvidence, "corrections" | "correctionCount" | "riskyCommands" | "steers">;
} {
  const tally: EvidenceTally = { corrections: [], risky: new Map(), steers: new Map(), tasked: new Set(), prompts: 0, recentCalls: [] };
  let currentSession = "";
  for (const event of stored) {
    if (event.sessionId !== currentSession) {
      currentSession = event.sessionId;
      tally.recentCalls = [];
    }
    const payload = payloadOf(event);
    if (event.kind === "call") tallyCall(tally, payload);
    else if (event.kind === "steering-delivered") tallySteer(tally, payload);
    else if (typeof payload.text === "string") tallyPrompt(tally, event, payload.text, hostOf.get(event.sessionId) ?? "unknown");
  }
  // Standing instructions first, then the newest: they are the ones most likely to be rules.
  const corrections = tally.corrections.sort((left, right) => Number(right.standing) - Number(left.standing) || right.at.localeCompare(left.at));
  return {
    prompts: tally.prompts,
    found: {
      corrections: corrections.slice(0, limit),
      correctionCount: corrections.length,
      riskyCommands: [...tally.risky.values()].sort((left, right) => right.count - left.count),
      steers: [...tally.steers].map(([ruleId, count]) => ({ ruleId, count }))
        .sort((left, right) => right.count - left.count || left.ruleId.localeCompare(right.ruleId)),
    },
  };
}

function countRisky(risky: Map<string, EvidenceCommand>, command: string): void {
  for (const { label, pattern } of RISKY_COMMANDS) {
    if (!pattern.test(command)) continue;
    const seen = risky.get(label);
    if (seen === undefined) risky.set(label, { label, count: 1, example: clip(command, CALL_TEXT_LENGTH) });
    else seen.count += 1;
  }
}

function correctionLines(evidence: RuleEvidence): string[] {
  const shown = evidence.corrections.length < evidence.correctionCount ? `, the first ${evidence.corrections.length} shown` : "";
  const lines = [`  Corrections: ${evidence.correctionCount}${shown}.`];
  for (const correction of evidence.corrections) {
    lines.push(`    ${correction.at.slice(0, 10)} ${correction.host} [${correction.families.join(", ")}] ${correction.text}`);
    for (const call of correction.before) lines.push(`      after: ${call}`);
  }
  return lines;
}

/** A heading, then one line per entry, or the heading alone ending in "none". */
function listLines(heading: string, entries: readonly string[]): string[] {
  return entries.length === 0 ? [`  ${heading}: none`] : [`  ${heading}:`, ...entries.map(entry => `    ${entry}`)];
}

export function formatRuleEvidence(evidence: RuleEvidence): string {
  const { repository, window } = evidence;
  const standards = `  Standards files: ${evidence.standards.length === 0 ? "none" : evidence.standards.join(", ")}`;
  if (!repository.known) return [`● Rerail has no sessions for ${repository.name} yet.`, standards].join("\n");
  const hosts = Object.entries(window.hosts).map(([host, count]) => `${host} ${count}`).join(", ");
  const sessionsText = `${window.sessions} ${window.sessions === 1 ? "session" : "sessions"}`;
  return [
    `● ${repository.name}: ${sessionsText} in the last ${window.days} days${hosts === "" ? "" : ` (${hosts})`}, ${window.prompts} prompts.`,
    ...correctionLines(evidence),
    ...listLines("Risky commands", evidence.riskyCommands.map(command => `${command.label} ×${command.count}  ${command.example}`)),
    ...listLines("Steers by rule", evidence.steers.map(steer => `${steer.ruleId} ×${steer.count}`)),
    standards,
  ].join("\n");
}

function positiveInteger(value: string | undefined, option: string): number | undefined {
  if (value === undefined) return undefined;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1) throw new Error(`${option} must be a positive whole number`);
  return parsed;
}

/** `rerail rules evidence [--repo <dir>] [--days <n>] [--limit <n>] [--json]`. */
export async function runRuleEvidenceCommand(
  args: readonly string[],
  usage: () => string,
  io: { stdout?: { write(text: string): unknown }; databasePath?: string } = {},
): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--json"], options: ["--repo", "--days", "--limit"] },
    argument => `Unknown rules evidence option: ${argument}\n\n${usage()}`,
  );
  const days = positiveInteger(options.get("--days"), "--days");
  const limit = positiveInteger(options.get("--limit"), "--limit");
  const evidence = await collectRuleEvidence(options.get("--repo") ?? process.cwd(), {
    ...(days === undefined ? {} : { days }),
    ...(limit === undefined ? {} : { limit }),
    ...(io.databasePath === undefined ? {} : { databasePath: io.databasePath }),
  });
  (io.stdout ?? process.stdout).write(`${flags.has("--json") ? JSON.stringify(evidence) : formatRuleEvidence(evidence)}\n`);
  return 0;
}
