import { existsSync } from "node:fs";
import { access, readdir } from "node:fs/promises";
import { join, resolve } from "node:path";
import { and, asc, gte, inArray } from "drizzle-orm";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import { followsGitignore, listRepositories, lookupCheckout } from "../engine/catalog/repository-registry.js";
import { loadNativeBinding, type NativeGitIgnoreMatcher } from "../engine/native/native.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { events, sessions } from "../engine/storage/schema.js";
import { callNamesDeniedPath } from "../engine/transcript-boundary.js";

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

/** Shell words after which a command starts: `then rm`, `! git push`. */
const SHELL_KEYWORDS = new Set(["then", "do", "else", "if", "while", "until", "!", "{"]);

/**
 * Wrappers that run the command after them (`sudo rm`, `nohup nice -n 10 rm`), each with the options that take the next
 * word as their value, short and long (`sudo -u root`, `sudo --user root`); `--user=root` is one word. Any other option stands alone, so `sudo -E foo rm` is `foo`'s, not a wrapped rm.
 * `command` is left out: `command -v sudo` looks a name up rather than running it.
 */
const WRAPPER_VALUE_OPTIONS: ReadonlyMap<string, ReadonlySet<string>> = new Map([
  ["sudo", new Set(["-u", "--user", "-g", "--group", "-C", "--close-from", "-D", "--chdir", "-h", "--host", "-p", "--prompt", "-r", "--role", "-t", "--type", "-T", "--command-timeout", "-U", "--other-user"])],
  ["xargs", new Set(["-I", "-n", "--max-args", "-P", "--max-procs", "-L", "-d", "--delimiter", "-s", "--max-chars", "-E", "-a", "--arg-file"])],
  ["exec", new Set(["-a"])],
  ["time", new Set(["-o", "--output", "-f", "--format"])],
  ["nice", new Set(["-n", "--adjustment"])],
  ["nohup", new Set<string>()],
  ["env", new Set(["-u", "--unset", "-C", "--chdir", "-S", "--split-string"])],
]);

/**
 * The simple commands of a shell line, as words: split at new lines, `;`, `&`, `|`, parentheses, and a `find -exec` or
 * `-execdir`. Quoted strings are blanked first, so `grep 'npm publish'` holds no command. It reads characters, not
 * shell: good enough to count habits, not to judge one command.
 */
export function simpleCommands(line: string): string[][] {
  const unquoted = line.replace(/'[^']*'|"(?:[^"\\]|\\.)*"/gu, "''");
  return unquoted.split(/[\n;&|()]+/u).flatMap(part => {
    const commands: string[][] = [[]];
    for (const word of part.trim().split(/\s+/u).filter(word => word !== "")) {
      if (word === "-exec" || word === "-execdir" || word === "-ok") commands.push([]);
      else commands.at(-1)!.push(word);
    }
    return commands.filter(words => words.length > 0);
  });
}

/** Reading the words before a command: the current wrapper's value options, and whether the next word is an option's value. */
interface PrefixState {
  valueOptions: ReadonlySet<string> | undefined;
  valueNext: boolean;
}

/** A shell keyword (`then`, `!`) or an env assignment (`CI=1`), which may stand before any command. */
function standsBeforeAnyCommand(word: string): boolean {
  return SHELL_KEYWORDS.has(word) || /^\w+=/u.test(word);
}

/** The state after one more word, or undefined when the word cannot lead up to a command (`echo`, `git`). */
function prefixStep(state: PrefixState, word: string): PrefixState | undefined {
  if (state.valueNext) return { ...state, valueNext: false };
  const wrapper = WRAPPER_VALUE_OPTIONS.get(word);
  if (wrapper !== undefined) return { valueOptions: wrapper, valueNext: false };
  if (state.valueOptions !== undefined && word.startsWith("-")) return { ...state, valueNext: state.valueOptions.has(word) };
  return standsBeforeAnyCommand(word) ? state : undefined;
}

/**
 * Whether every word before `index` only leads up to the command there: a shell keyword, an env assignment, a wrapper,
 * one of the wrapper's options, or the value of an option that takes one.
 */
function onlyPrefixes(words: readonly string[], index: number): boolean {
  let state: PrefixState | undefined = { valueOptions: undefined, valueNext: false };
  for (const word of words.slice(0, index)) {
    state = prefixStep(state, word);
    if (state === undefined) return false;
  }
  return !state.valueNext;
}

/** Commands worth a conversation rule wherever they show up. Each `test` reads one simple command from its command word on. */
export const RISKY_COMMANDS: ReadonlyArray<{ label: string; commands: readonly string[]; test: RegExp }> = [
  { label: "force push", commands: ["git"], test: /^git\s+push\b.*\s(?:--force(?:\s|=|$)|-[A-Za-z]*f[A-Za-z]*(?:\s|$))/u },
  // `-n` skips hooks only for `git commit`: for push it is --dry-run, for merge --no-stat.
  {
    label: "skip git hooks",
    commands: ["git"],
    test: /^git\s+commit\b.*\s-[A-Za-z]*n[A-Za-z]*(?:\s|$)|^git\s+(?:commit|push|merge|rebase)\b.*\s--no-verify(?:\s|=|$)/u,
  },
  { label: "hard reset", commands: ["git"], test: /^git\s+reset\b.*\s--hard(?:\s|$)/u },
  { label: "discard changes", commands: ["git"], test: /^git\s+(?:clean\b.*\s-[A-Za-z]*f|checkout\s+(?:--|\.)(?:\s|$)|restore\s+(?:--worktree\s+)?\.(?:\s|$))/u },
  // Both flags, in any case, order or grouping: `-rf`, `-Rf`, `-r -f`, `--recursive --force`.
  { label: "recursive delete", commands: ["rm"], test: /^rm(?=.*\s(?:-[A-Za-z]*[rR][A-Za-z]*|--recursive)(?:\s|$))(?=.*\s(?:-[A-Za-z]*f[A-Za-z]*|--force)(?:\s|$))/u },
  { label: "publish", commands: ["npm", "pnpm", "yarn", "cargo", "twine"], test: /^(?:(?:npm|pnpm|yarn|cargo)\s+publish|twine\s+upload)(?:\s|$)/u },
  { label: "sudo", commands: ["sudo"], test: /^sudo(?:\s|$)/u },
];

/** The labels one simple command earns, each read from its command word on. */
function commandLabels(words: readonly string[]): string[] {
  return RISKY_COMMANDS.filter(({ commands, test }) => {
    const index = words.findIndex(word => commands.includes(word));
    return index !== -1 && onlyPrefixes(words, index) && test.test(words.slice(index).join(" "));
  }).map(({ label }) => label);
}

/** The risky labels a shell line earns: per simple command, from its command word, plus a download piped into a shell. */
export function riskyLabels(line: string): string[] {
  const labels = new Set(simpleCommands(line).flatMap(commandLabels));
  // A pipe joins two commands, so this one reads the line: `curl … | sh`.
  if (/\b(?:curl|wget)\b[^\n|]*\|\s*(?:sudo\s+)?(?:ba|z)?sh\b/u.test(line)) labels.add("pipe to shell");
  return [...labels];
}

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
  // Typographic apostrophes (macOS, phones) read as plain ones: "Don’t" is "Don't".
  const trimmed = text.trim().replace(/[‘’]/gu, "'");
  if (trimmed.length === 0 || trimmed.startsWith("<") || trimmed.startsWith("#") || trimmed.startsWith("---")) return [];
  if (trimmed.split(/\s+/).length > CORRECTION_MAX_WORDS) return [];
  return CORRECTION_FAMILIES.filter(family => family.pattern.test(trimmed)).map(family => family.name);
}

/**
 * `<tool> <command or path>`: what a call did, in one line. A call that names a hidden or secret-like path (`cat
 * .env`) is named by its tool alone: the evidence goes to a coding agent, and that path stays out of its conversation.
 */
function callSummary(payload: Record<string, unknown>): string {
  const tool = typeof payload.toolName === "string" ? payload.toolName : "tool";
  if (callNamesDeniedPath(payload.input)) return `${tool} (a hidden or secret-like path, withheld)`;
  const input = typeof payload.input === "object" && payload.input !== null ? payload.input as Record<string, unknown> : {};
  const target = [input.command, input.file_path, input.path, input.filePath, input.pattern].find(value => typeof value === "string");
  return clip(typeof target === "string" ? `${tool} ${target}` : tool, CALL_TEXT_LENGTH);
}

async function exists(path: string): Promise<boolean> {
  return access(path).then(() => true, () => false);
}

/**
 * Whether Git ignores a path under `root`, for a repository that follows `.gitignore`: a standards file Git ignores is
 * the person's own, not the project's, so the evidence does not send the agent to it. In a Git checkout whose rules
 * cannot be read, every path counts as ignored rather than guessed at.
 */
function gitIgnoredUnder(root: string, follows: boolean): (path: string) => boolean {
  if (!follows) return () => false;
  let matcher: NativeGitIgnoreMatcher | null;
  try {
    matcher = loadNativeBinding().openGitIgnoreMatcher(root);
  } catch {
    return () => true;
  }
  if (matcher === null) return () => existsSync(join(root, ".git"));
  const opened = matcher;
  return path => {
    try {
      return opened.ignored(path, false);
    } catch {
      return true;
    }
  };
}

async function standardsFiles(root: string, ignored: (path: string) => boolean): Promise<string[]> {
  const files = await Promise.all(STANDARDS_FILES.map(async file => (await exists(join(root, file)) ? [file] : [])));
  const directories = await Promise.all(STANDARDS_DIRECTORIES.map(async directory => {
    const entries = await readdir(join(root, directory), { withFileTypes: true }).catch(() => []);
    return entries.filter(entry => entry.isFile()).map(entry => `${directory}/${entry.name}`);
  }));
  return [...files, ...directories].flat().filter(path => !ignored(path)).sort();
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
  // Standards files sit at the checkout root, wherever under it the command runs.
  const root = repositoryIdentityLocation(resolve(repo)).workspaceRoot;
  const name = repositoryName(root);
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  if (!(await exists(databasePath))) return emptyEvidence(name, days, await standardsFiles(root, gitIgnoredUnder(root, true)));
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const checkout = lookupCheckout(database, root);
    const standards = await standardsFiles(root, gitIgnoredUnder(root, followsGitignore(database, checkout)));
    if (checkout === undefined) return emptyEvidence(name, days, standards);
    // A session keeps the repository key it opened under: an alias the repository has since adopted, or a checkout's
    // clone id from before the repository had a remote. Its history is all of them, as `rerail reset` reads it.
    const repository = listRepositories(database).find(candidate => candidate.repositoryKey === checkout.repositoryKey);
    const historyIds = [...new Set([checkout.repositoryKey, ...(repository?.aliases ?? []), ...(repository?.checkouts ?? []).map(row => row.cloneId)])];
    const inWindow = and(inArray(sessions.repositoryId, historyIds), gte(sessions.startedAt, since));
    const repositorySessions = database.select({ sessionId: sessions.sessionId, host: sessions.host }).from(sessions).where(inWindow).all();
    const hostOf = new Map(repositorySessions.map(session => [session.sessionId, session.host]));
    const hosts: Record<string, number> = {};
    for (const { host } of repositorySessions) hosts[host] = (hosts[host] ?? 0) + 1;
    const stored = database.select({
      sessionId: events.sessionId, kind: events.kind, observedAt: events.observedAt, payloadJson: events.payloadJson,
    }).from(events)
      .where(and(
        inArray(events.sessionId, database.select({ sessionId: sessions.sessionId }).from(sessions).where(inWindow)),
        inArray(events.kind, ["user-prompt", "call", "steering-delivered"]),
      ))
      .orderBy(asc(events.sessionId), asc(events.sequence)).all();
    const evidence = readEvents(stored, hostOf, limit);
    return {
      repository: { name, known: true, checkouts: repository?.checkouts.length ?? 1 },
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
  // A risky command's example is quoted to the agent; one over a path outside the boundary is not.
  if (typeof input.command === "string" && !callNamesDeniedPath(input.command)) countRisky(tally.risky, input.command);
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
  for (const label of riskyLabels(command)) {
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
