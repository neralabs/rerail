import type { SetupOption, SetupOptionInput, SetupQuestion, SetupQuestionId } from "./setup-step.js";
import { RERAIL_BRAND_FOREGROUND } from "../presentation/terminal-brand.generated.js";
import { rerailMark, type ColourDepth } from "../presentation/rerail-mark.js";

/**
 * The text of every install and set-up screen (T2–T6, C3–C6b, WR,
 * E1–E6), as pure functions of plain data.
 *
 * Three lanes. `terminal` is a person at a TTY: brand colour, dimmed hints, a
 * cursor on the first answer. `agent` is set-up run by a coding agent (no
 * TTY): plain text addressed to the coding agent, which relays each question
 * to the person, waits, and runs set-up again with the answer. `json` is the
 * agent lane for a coding agent's own UI (`--json`): plain text for the person,
 * without the relay instructions; the question goes out as data (`setup-step.ts`).
 */
export type SetupLane = "terminal" | "agent" | "json";

export interface CodingAgent {
  /** The value `--agents` takes, e.g. `claude`. */
  readonly id: string;
  /** The name people see, e.g. `Claude Code`. */
  readonly name: string;
}

export interface RecommendedPack {
  /** The pack's id, which `--packs` names it by. */
  readonly id: string;
  readonly name: string;
  readonly ruleCount: number;
  readonly reason: string;
  /** The npm package and exact version set-up installs once the person says yes. */
  readonly packageName: string;
  readonly version: string;
  /** The version installed now, if any. Every repository on this computer runs it, so installing another replaces it there too. */
  readonly installed: string | null;
}

/** A rule pack "Choose rule packs…" offers: every one set-up fetched, matched or not. */
export interface AvailablePack {
  readonly id: string;
  readonly version: string;
  readonly name: string;
  /** Why it fits the repository; undefined for a pack that does not. */
  readonly reason?: string;
}

export interface DashboardLink {
  readonly url: string;
  readonly browserOpened: boolean;
  /** Why no browser opened, e.g. "this looks like a remote session". */
  readonly noBrowserReason?: string;
  /** Set-up did not try to open a browser (an earlier run did): the link alone, with no E6 line. */
  readonly linkOnly?: boolean;
}

/** The command a coding agent runs to answer a question; set-up's own invocation. */
export const SETUP_COMMAND = "rerail setup";

/**
 * The flags the agent lane's answer commands use; set-up's argument parser reads the same ones. `relay` picks the
 * agent lane even when a coding agent runs commands in a terminal: `/rerail-start` and every answer pass it.
 */
export const SETUP_FLAGS = { answer: "--answer", repo: "--repo", agents: "--agents", packs: "--packs", relay: "--relay" } as const;

/** Answer keys in the order each question numbers its answers (answer 1 is index 0). */
export const READ_REPO_ANSWERS = ["yes", "other-folder", "pick-myself"] as const;
export const CONFIRM_PACKS_ANSWERS = ["start", "choose", "not-now"] as const;
export const NO_REPO_ANSWERS = ["folder", "pick-myself", "skip"] as const;
/** "I'll pick rule packs myself": pick some of every pack set-up fetched, or start with none. */
export const PICK_PACKS_ANSWERS = ["choose", "none"] as const;
/** `--packs none`: the answer that starts Rerail with no rule pack. */
export const NO_PACKS = "none";
export const SESSIONS_ANSWERS = ["allow", "decline"] as const;
export type ReadRepoAnswer = (typeof READ_REPO_ANSWERS)[number];
export type ConfirmPacksAnswer = (typeof CONFIRM_PACKS_ANSWERS)[number];
export type NoRepoAnswer = (typeof NO_REPO_ANSWERS)[number];
export type SessionsAnswer = (typeof SESSIONS_ANSWERS)[number];

// ── Styling ─────────────────────────────────────────────────────────────────

interface Style {
  brand(text: string): string;
  dim(text: string): string;
  bold(text: string): string;
  red(text: string): string;
  yellow(text: string): string;
}

const ESC = "\u001b[";
const TERMINAL_STYLE: Style = {
  brand: text => `${RERAIL_BRAND_FOREGROUND}${text}${ESC}39m`,
  dim: text => `${ESC}2m${text}${ESC}22m`,
  bold: text => `${ESC}1m${text}${ESC}22m`,
  red: text => `${ESC}31m${text}${ESC}39m`,
  yellow: text => `${ESC}33m${text}${ESC}39m`,
};
const plain = (text: string): string => text;
const PLAIN_STYLE: Style = { brand: plain, dim: plain, bold: plain, red: plain, yellow: plain };

const styleFor = (lane: SetupLane): Style => (lane === "terminal" ? TERMINAL_STYLE : PLAIN_STYLE);

/** Repo names, reasons and paths come from disk; never let them carry escape sequences. */
export const clean = (value: string): string => value.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu, "");

const MASCOT_HAPPY = ">^ᴥ^<";
const LABEL_WIDTH = 15;
const COMMAND_WIDTH = 17;
const MODE_WIDTH = 8;
const MODE_TEXT_WIDTH = 37;
const TABLE_RULES_WIDTH = 7;
const TABLE_MIN_PACK_WIDTH = 16;
const SUMMARY_LABEL_WIDTH = 10;
const TIPS_DIVIDER = "── Good to know ─────────────────────────────────────────────";

const lines = (rows: readonly string[]): string => `${rows.join("\n")}\n`;
const withHint = (text: string, hint: string | undefined, style: Style): string =>
  hint === undefined ? text : `${text}  ${style.dim(hint)}`;
const labelled = (label: string, value: string, style: Style, hint?: string): string =>
  `  ${label.padEnd(LABEL_WIDTH)}${withHint(value, hint, style)}`;
const bullet = (text: string, style: Style): string => `${style.brand("●")} ${text}`;

function plural(count: number, noun: string): string {
  return `${count} ${count === 1 ? noun : `${noun}s`}`;
}

/** "Claude Code", "Claude Code and Cursor", "Claude Code, Cursor and Codex". */
function andList(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
}

function agentNames(agents: readonly CodingAgent[]): string[] {
  return agents.map(agent => clean(agent.name));
}

function connectedText(agents: readonly CodingAgent[]): string {
  return agents.length === 0 ? "no coding agents found" : andList(agentNames(agents));
}

const SHELL_SAFE = /^[\w@%+=:,./~-]+$/;
/**
 * One shell word; set-up quotes the launcher path it hands the coding agent with it too. POSIX shells get single
 * quotes. Windows gets double quotes, as cmd.exe (and Git Bash) take a quoted path, like every command Rerail writes
 * for Windows (`harness-registration.ts`); a Windows path cannot contain `"`.
 */
export function shellQuote(value: string, platform: NodeJS.Platform = process.platform): string {
  if (SHELL_SAFE.test(value)) return value;
  return platform === "win32" ? `"${value}"` : `'${value.replaceAll("'", "'\\''")}'`;
}

// ── Questions ───────────────────────────────────────────────────────────────

/** What an answer asks the person for, plus what the agent lane writes in its place. */
interface ChoiceInput extends SetupOptionInput {
  readonly placeholder: string;
}

interface Choice {
  /** The answer key `--answer` takes for it; also what `--json` reports. */
  readonly answer: string;
  readonly label: string;
  readonly hint?: string;
  /** The arguments after `setup` that give this answer. */
  readonly args: readonly string[];
  /** A value the person names first (a folder, coding agent ids), appended as `[flag, value]`. */
  readonly input?: ChoiceInput;
  /** Agent lane: the sentence around the command, when the command alone does not say enough. */
  readonly agentText?: (command: string) => string;
}

/** One question, in the parts every lane needs: the lines around the question, the question, and its answers. */
export interface QuestionForm {
  readonly id: SetupQuestionId;
  readonly lead: readonly string[];
  readonly prompt: string;
  readonly trail: readonly string[];
  readonly choices: readonly Choice[];
  /** Overrides `rerail setup` in the agent lane's answer commands. */
  readonly setupCommand?: string;
}

const AGENT_QUESTION_INTRO = "Rerail set-up is waiting for the person’s answer. Ask them this, as written, and wait for their reply:";
const AGENT_QUESTION_OUTRO = "Do not choose for them. When they answer, run set-up again with their answer:";

/** What the coding agent runs for one answer: the command, the answer's arguments, and the value it names. */
function agentCommand(form: QuestionForm, choice: Choice): string {
  const input = choice.input === undefined ? [] : [choice.input.flag, choice.input.placeholder];
  const command = [form.setupCommand ?? SETUP_COMMAND, ...choice.args.map(arg => shellQuote(arg)), ...input].join(" ");
  return choice.agentText === undefined ? command : choice.agentText(command);
}

/** Where a question sits in the set-up's main flow, shown before it in the terminal: "Step 2 of 3". */
export interface QuestionStep {
  readonly index: number;
  readonly total: number;
}

/**
 * Terminal: the step, the body, then the answers with a cursor on the first.
 * Agent: the body and answers as text to relay, then one command per answer.
 * Json: the body without the question line, which `setupQuestion` carries with the answers.
 */
export function renderQuestion(form: QuestionForm, lane: SetupLane, step?: QuestionStep): string {
  const style = styleFor(lane);
  if (lane === "json") return lines([...form.lead, ...form.trail]);
  const prompt = lane === "terminal" && step !== undefined ? `Step ${step.index} of ${step.total} · ${form.prompt}` : form.prompt;
  const body = [...form.lead, ...(form.lead.length === 0 ? [] : [""]), style.bold(prompt), ...form.trail];
  // The hints line up in one column, past the longest answer.
  const width = Math.max(...form.choices.map((choice, index) => `${index + 1}. ${choice.label}`.length)) + 4;
  const answer = (choice: Choice, index: number): string => {
    const text = `${index + 1}. ${choice.label}`;
    return choice.hint === undefined ? text : `${text.padEnd(width)}${style.dim(choice.hint)}`;
  };
  if (lane === "terminal") {
    // A blank line first, so each question stands apart from what the answer before it printed.
    return lines(["", ...body, "", ...form.choices.map((choice, index) => `${index === 0 ? style.brand("❯") : " "} ${answer(choice, index)}`)]);
  }
  return lines([
    AGENT_QUESTION_INTRO,
    "",
    ...body,
    "",
    ...form.choices.map(answer),
    "",
    AGENT_QUESTION_OUTRO,
    ...form.choices.map((choice, index) => `${index + 1}. ${agentCommand(form, choice)}`),
  ]);
}

/** The question as `--json` reports it. `baseArgs` start every answer's arguments: `setup`, `--port`, `--json`. */
export function setupQuestion(form: QuestionForm, baseArgs: readonly string[]): SetupQuestion {
  return {
    id: form.id,
    prompt: form.prompt,
    options: form.choices.map((choice): SetupOption => ({
      answer: choice.answer,
      label: choice.label,
      detail: choice.hint ?? null,
      next: { args: [...baseArgs, ...choice.args] },
      input: choice.input === undefined ? null : { flag: choice.input.flag, prompt: choice.input.prompt, hint: choice.input.hint },
    })),
  };
}

const FOLDER_PLACEHOLDER = "<the folder they name>";
const folderInput = (example: string): ChoiceInput => ({ flag: SETUP_FLAGS.repo, prompt: "Which folder?", hint: `for example ${example}`, placeholder: FOLDER_PLACEHOLDER });

// ── T1 · Banner ────────────────────────────────────────────────────────────

export interface BannerInput {
  readonly version: string;
  /** What the terminal can draw (`colourDepth`); `none` keeps the letters plain. */
  readonly depth: ColourDepth;
}

/** T1: the footer's `R E R A I L` mark, in its colours, heading a set-up run in a terminal. Other lanes have none. */
export function renderBanner(input: BannerInput, lane: SetupLane): string {
  if (lane !== "terminal") return "";
  const style = input.depth === "none" ? PLAIN_STYLE : TERMINAL_STYLE;
  return lines(["", `  ${rerailMark(input.depth)}   ${style.dim(`set-up · ${clean(input.version)}`)}`, ""]);
}

// ── T2 · Installed ─────────────────────────────────────────────────────────

/** T2's finished line (terminal), under the banner that names the version; the line C3 opens with (agent, json). */
export function renderInstalled(lane: SetupLane): string {
  if (lane !== "terminal") return lines(["Rerail is installed. Nothing is on yet, and your code is untouched."]);
  const style = styleFor(lane);
  return lines([`${style.brand(MASCOT_HAPPY)}  Rerail installed ${style.dim("· nothing is on yet, your code is untouched")}`]);
}

// ── T2 · On PATH ───────────────────────────────────────────────────────────

export interface OnPathInput {
  /** The launcher's directory, `~/.rerail/bin`. */
  readonly binDir: string;
  /** The startup files (or the Windows user Path) set-up added it to; empty when it could not. */
  readonly locations: readonly string[];
  /** Why set-up could not add it, when it could not. */
  readonly failure?: string;
  /** The person's home, shown as `~`. */
  readonly home: string;
}

/** The line after T2 when set-up put the launcher on PATH, or could not. */
export function renderOnPath(input: OnPathInput, lane: SetupLane): string {
  const style = styleFor(lane);
  const home = input.home.replace(/[\\/]+$/u, "");
  const tilde = (path: string): string =>
    clean(home !== "" && path.startsWith(home) && /^[\\/]/u.test(path.slice(home.length)) ? `~${path.slice(home.length)}` : path);
  const binDir = tilde(input.binDir);
  if (input.failure !== undefined) {
    return lines([`${style.yellow("△")} Couldn’t add ${binDir} to PATH (${clean(input.failure)}). Add it yourself to run rerail from a terminal.`]);
  }
  const where = input.locations.map(tilde).join(" and ");
  if (lane !== "terminal") return lines([`Added ${binDir} to PATH in ${where}. Open a new terminal to run rerail.`]);
  return lines([bullet("You can run `rerail` from any new terminal", style)]);
}

// ── T2b · May Rerail read sessions ─────────────────────────────────────────

export interface SessionsInput {
  /** What every answer carries on, so the run picks up where it was: `--repo <folder>` when one was named. */
  readonly carriedArgs: readonly string[];
  readonly setupCommand?: string;
}

/** T2b: asked once on this computer, before set-up connects anything, so the person agrees to what Rerail reads. */
export function sessionsQuestion(input: SessionsInput): QuestionForm {
  return {
    id: "sessions",
    lead: [],
    prompt: "Can Rerail see your coding agent sessions?",
    trail: [
      "Rerail sees what you ask your agent, what it does, and the files it edits,",
      "so it can check the work against your rules.",
      "",
      "  ✓ Stays on this computer. Nothing is sent anywhere.",
      "  ✓ You’re asked once, for every repo you start Rerail in.",
    ],
    choices: [
      { answer: "allow", label: "Yes, allow", args: [SETUP_FLAGS.answer, "allow", ...input.carriedArgs] },
      { answer: "decline", label: "No, cancel set-up", hint: "nothing changes", args: [SETUP_FLAGS.answer, "decline"] },
    ],
    ...(input.setupCommand === undefined ? {} : { setupCommand: input.setupCommand }),
  };
}

/** After "No": nothing new is connected or started. Coding agents connected before stay connected until removed. */
export function renderSessionsDeclined(lane: SetupLane): string {
  const style = styleFor(lane);
  return lines([
    ...(lane === "terminal" ? [""] : []),
    bullet("Set-up stopped. Nothing new was connected or started.", style),
    `  ${style.dim("Rerail needs your sessions to check rules. Run set-up again whenever you want to allow it.")}`,
  ]);
}

// ── T3 / C3 · Ask before reading ───────────────────────────────────────────

export interface ReadRepoInput {
  readonly repoName: string;
  readonly repoPath: string;
  /** Overrides `rerail setup` in the agent lane's answer commands. */
  readonly setupCommand?: string;
}

/** T3 / C3: Rerail names the repo, says what scanning it does, and asks before reading it. */
export function readRepoQuestion(input: ReadRepoInput): QuestionForm {
  const repo = clean(input.repoName);
  const repoFlag = [SETUP_FLAGS.repo, clean(input.repoPath)];
  return {
    id: "read-repo",
    lead: [],
    prompt: `Find rule packs for ${repo}?`,
    trail: [
      "Rule packs are ready-made sets of rules your coding agent must follow.",
      "Rerail can scan this repo’s files and suggest packs that fit.",
      "",
      "  ✓ Scanning only reads files. Your code stays on this computer.",
      "  ✓ Nothing is installed or turned on until you say yes.",
    ],
    choices: [
      { answer: "yes", label: `Yes, scan ${repo}`, args: [SETUP_FLAGS.answer, "yes", ...repoFlag] },
      { answer: "other-folder", label: "Scan a different repo…", hint: "e.g. ~/code/landing-page", args: [], input: folderInput("~/code/landing-page") },
      { answer: "pick-myself", label: "Skip, I’ll pick packs", hint: "Rerail won’t scan", args: [SETUP_FLAGS.answer, "pick-myself", ...repoFlag] },
    ],
    ...(input.setupCommand === undefined ? {} : { setupCommand: input.setupCommand }),
  };
}

export function renderReadRepoQuestion(input: ReadRepoInput, lane: SetupLane): string {
  return renderQuestion(readRepoQuestion(input), lane);
}

// ── T4 / C4 · Recommended rule packs and the confirm question ──────────────

/** What set-up takes from the repository's own lint config: `repository-lint-config.ts`'s result. */
export interface LintConfigInput {
  /** Rules that start off (`on: false`) or with option values (`config`), in a folder or everywhere (`""`), and the config entry that says so. */
  readonly settings: readonly {
    readonly ruleId: string;
    readonly folder: string;
    readonly on: false | null;
    readonly config: Readonly<Record<string, unknown>> | null;
    readonly because: string;
  }[];
  /** Files not read, options not mapped, conflicts. */
  readonly notes: readonly string[];
}

export interface RecommendedPacksInput {
  readonly repoName: string;
  readonly repoPath: string;
  readonly packs: readonly RecommendedPack[];
  /** Every rule pack "Choose rule packs…" offers, the recommended ones first. */
  readonly available: readonly AvailablePack[];
  /** Rule packs set-up could not fetch or check, and why. */
  readonly unavailable?: readonly string[];
  /** The coding agents "Yes" connects. */
  readonly agentsFound: readonly CodingAgent[];
  readonly setupCommand?: string;
  /** The rule settings starting Rerail takes from the repository's lint config; nothing shown when empty. */
  readonly lintConfig?: LintConfigInput;
}

/** At most this many lines say which rule packs set-up could not fetch; the rest are counted. */
const MAX_UNAVAILABLE_LINES = 4;

/** At most this many lint config lines; the rest are counted. */
const MAX_LINT_CONFIG_LINES = 8;

/** Which rules start off or configured from the repository's lint config, and what was not read. */
function lintConfigLines(input: RecommendedPacksInput, style: Style): string[] {
  const settings = input.lintConfig?.settings ?? [];
  const notes = input.lintConfig?.notes ?? [];
  if (settings.length === 0 && notes.length === 0) return [];
  // What was not read comes first: it can explain every setting after it (a config not statically readable starts
  // every rule it ports off), and the rows are capped.
  const rows = [
    ...notes.map(note => `    ${style.dim(clean(note))}`),
    ...settings.map(setting => {
      const options = Object.entries(setting.config ?? {}).map(([name, value]) => `${name} = ${JSON.stringify(value)}`).join(", ");
      const change = setting.on === false ? (setting.folder === "" ? "starts off" : `starts off in ${clean(setting.folder)}/`) : `starts with ${options}`;
      return `    ${clean(setting.ruleId)} ${change}  ${style.dim(clean(setting.because))}`;
    }),
  ];
  const shown = rows.length > MAX_LINT_CONFIG_LINES ? rows.slice(0, MAX_LINT_CONFIG_LINES - 1) : rows;
  const more = rows.length - shown.length;
  return [
    "  From your lint config (read as data, never run):",
    ...shown,
    ...(more === 0 ? [] : [`    …and ${more} more.`]),
  ];
}

function packTable(packs: readonly RecommendedPack[], style: Style): string[] {
  const names = packs.map(pack => clean(pack.name));
  const nameWidth = Math.max(TABLE_MIN_PACK_WIDTH, ...names.map(name => name.length + 2));
  const header = style.dim(`  ${"Rule pack".padEnd(nameWidth)}${"Rules".padEnd(TABLE_RULES_WIDTH)}Why it fits`);
  const rows = packs.map(
    (pack, index) => `  ${(names[index] ?? "").padEnd(nameWidth)}${String(pack.ruleCount).padEnd(TABLE_RULES_WIDTH)}${clean(pack.reason)}`,
  );
  return [header, ...rows];
}

/**
 * The packs "Yes" would install over another version: every repository on this computer runs the installed one, so
 * the person sees that before saying yes.
 */
function replaceLines(packs: readonly RecommendedPack[], style: Style): string[] {
  return packs.flatMap(pack => pack.installed === null || pack.installed === pack.version
    ? []
    : [`  ${style.yellow("△")} ${clean(pack.packageName)}@${clean(pack.version)} replaces ${clean(pack.installed)}, which every repo on this computer uses.`]);
}

function unavailableLines(unavailable: readonly string[] | undefined, style: Style): string[] {
  if (unavailable === undefined || unavailable.length === 0) return [];
  const shown = unavailable.length > MAX_UNAVAILABLE_LINES ? unavailable.slice(0, MAX_UNAVAILABLE_LINES - 1) : unavailable;
  const more = unavailable.length - shown.length;
  return [
    "  Rule packs set-up could not use:",
    ...shown.map(problem => `    ${style.dim(clean(problem))}`),
    ...(more === 0 ? [] : [`    …and ${more} more.`]),
  ];
}

function recommendationLines(input: RecommendedPacksInput, lane: SetupLane): string[] {
  const style = styleFor(lane);
  const repo = clean(input.repoName);
  const count = input.packs.length;
  const scanned = count === 0
    ? `Scanned ${repo}. None of Rerail’s rule packs fit it.`
    : `Scanned ${repo}. ${count === 1 ? "1 rule pack fits" : `${count} rule packs fit`}:`;
  const connecting = input.agentsFound.length === 0
    ? ["  Will connect to: no coding agents found"]
    : [`  Will connect to: ${agentNames(input.agentsFound).join(", ")}`, "  Adds /rerail commands and a status line to each. Connecting is free."];
  return [
    lane === "terminal" ? bullet(scanned, style) : scanned,
    ...(count === 0 ? [] : ["", ...packTable(input.packs, style)]),
    ...replaceLines(input.packs, style),
    ...lintConfigLines(input, style),
    ...unavailableLines(input.unavailable, style),
    "",
    ...connecting,
  ];
}

/** `typescript@0.1.1,complexity@0.1.1`: what an answer installs, carried as `--packs`. */
const packList = (packs: readonly { readonly id: string; readonly version: string }[]): string =>
  packs.map(pack => `${clean(pack.id)}@${clean(pack.version)}`).join(",");

/**
 * The confirm question's answers, in its order: no "Choose rule packs…" when set-up has no pack to choose from, since
 * nothing could be typed there and the person could not get back.
 */
export function confirmPacksAnswers(available: readonly unknown[]): readonly ConfirmPacksAnswer[] {
  return available.length === 0 ? CONFIRM_PACKS_ANSWERS.filter(answer => answer !== "choose") : CONFIRM_PACKS_ANSWERS;
}

function startQuestion(input: RecommendedPacksInput): string {
  const repo = clean(input.repoName);
  const count = input.packs.length;
  if (count === 0) return `Start Rerail in ${repo} with no rule packs on?`;
  return `Start Rerail in ${repo} with ${count === 1 ? "this rule pack" : `these ${count} rule packs`}?`;
}

/** T4 / C4: the rule packs that fit, each with its rule count and reason, then the confirm question. */
export function recommendedPacksQuestion(input: RecommendedPacksInput, lane: SetupLane): QuestionForm {
  const repoFlag = [SETUP_FLAGS.repo, clean(input.repoPath)];
  // "Yes" carries the exact versions shown, so a later run installs what the person saw.
  const packsFlag = input.packs.length === 0 ? [] : [SETUP_FLAGS.packs, packList(input.packs)];
  const available = packList(input.available);
  return {
    id: "confirm-packs",
    lead: recommendationLines(input, lane),
    prompt: startQuestion(input),
    trail: [
      "When your coding agent breaks a rule, Rerail tells it, and the agent fixes",
      "its own work. Rerail never edits your code itself.",
    ],
    choices: ([
      { answer: "start", label: "Yes, start Rerail", args: [SETUP_FLAGS.answer, "start", ...repoFlag, ...packsFlag] },
      {
        answer: "choose",
        label: "Change rule packs…",
        hint: "here or in the dashboard",
        args: [SETUP_FLAGS.answer, "choose", ...repoFlag],
        input: { flag: SETUP_FLAGS.packs, prompt: "Which rule packs should Rerail install?", hint: `comma-separated, from: ${available}`, placeholder: "<packs>" },
        agentText: command => `To choose the rule packs: ${command}, with comma-separated packs from: ${available}.`,
      },
      { answer: "not-now", label: "Not now", hint: "nothing is turned on", args: [SETUP_FLAGS.answer, "not-now"] },
    ] satisfies Choice[]).filter(choice => confirmPacksAnswers(input.available).includes(choice.answer as ConfirmPacksAnswer)),
    ...(input.setupCommand === undefined ? {} : { setupCommand: input.setupCommand }),
  };
}

export function renderRecommendedPacks(input: RecommendedPacksInput, lane: SetupLane): string {
  return renderQuestion(recommendedPacksQuestion(input, lane), lane);
}

// ── T5 · Not now ───────────────────────────────────────────────────────────

/** After "Not now": nothing was installed, connected or turned on. */
export function renderNotNow(lane: SetupLane): string {
  const style = styleFor(lane);
  const start = lane === "terminal" ? "rerail start" : "/rerail-start";
  return lines([
    ...(lane === "terminal" ? [""] : []),
    bullet("Set-up stopped. Nothing was installed or turned on.", style),
    `  ${style.dim(`Start Rerail in this repo whenever you’re ready with ${start}.`)}`,
  ]);
}

// ── T6 / C6 / E6 · Summary ─────────────────────────────────────────────────

/** "Manage rules" (or "Review") and its link. E6: the link is always shown, with one line when no browser opened. */
function linkLines(label: string, link: DashboardLink, style: Style): string[] {
  const url = clean(link.url);
  if (link.browserOpened) return [labelled(label, url, style, "opening in your browser")];
  if (link.linkOnly === true) return [labelled(label, url, style, "open this link")];
  const reason = link.noBrowserReason === undefined ? "" : ` (${clean(link.noBrowserReason)})`;
  return [labelled(label, url, style, "open this link"), `  ${style.yellow("△")} Couldn’t open a browser from here${reason}.`];
}

const YOUR_OWN_HINT = "create a rule in your own words, in a session";
const SHOW_ME_HINT = "see Rerail catch and fix a broken rule";

export interface SummaryInput {
  readonly repoName: string;
  /** The rule packs now on, by name. Never the rules themselves. */
  readonly packNames: readonly string[];
  readonly ruleCount: number;
  readonly dashboard: DashboardLink;
  /** Other repos where Rerail is already started; empty on a first set-up. */
  readonly otherStartedRepos?: readonly string[];
}

/** A summary row: its label, then its lines, each after the first under the one before. */
function summaryRow(label: string, rows: readonly string[]): string[] {
  return rows.map((row, index) => `  ${(index === 0 ? label : "").padEnd(SUMMARY_LABEL_WIDTH)}${row}`);
}

/** The dashboard link, and whether a browser opened it. E6: the link is always shown, with one line when none opened. */
function manageRows(link: DashboardLink, style: Style): string[] {
  const url = clean(link.url);
  if (link.browserOpened) return [url, style.dim("Opening in your browser.")];
  if (link.linkOnly === true) return [url];
  const reason = link.noBrowserReason === undefined ? "" : ` (${clean(link.noBrowserReason)})`;
  return [url, `${style.yellow("△")} Couldn’t open your browser${reason}. Copy the link instead.`];
}

function otherReposLine(repo: string, others: readonly string[]): string {
  const where = others.length === 0 ? `Rerail is only on in ${repo}.` : `Rerail is also on in ${andList(others.map(clean))}.`;
  return `  ${where} To add another repo, type /rerail-start there.`;
}

function summaryLines(input: SummaryInput, style: Style): string[] {
  const repo = clean(input.repoName);
  const packCount = input.packNames.length;
  // No rule pack fit the repository: say so plainly; the dashboard link is where to add one.
  const packsOn = packCount === 0 ? "no rule packs on yet" : `${plural(packCount, "rule pack")}, ${plural(input.ruleCount, "rule")}`;
  return [
    bullet(`Rerail is on in ${repo} · ${packsOn}`, style),
    "",
    ...summaryRow("Try it", ["Open a new agent session and type /rerail-show-me", "to watch Rerail catch and fix a broken rule."]),
    ...summaryRow("Manage", manageRows(input.dashboard, style)),
    "",
    otherReposLine(repo, input.otherStartedRepos ?? []),
    "  Type /rerail in any session for help and every command.",
    "",
  ];
}

const AGENT_SUMMARY_INTRO = "Rerail set-up is done. Show the person this summary, as written:";
const AGENT_TIPS_INTRO = "Only if they ask for tips, tell them this, as written:";

/**
 * T6 / C6. Everything needed sits at or above the link. Terminal: the tips
 * follow below a divider, dimmed. Agent: one line offers the tips (C6b), and
 * the tips follow for the coding agent to use only if the person asks.
 * Json: as the terminal shows it, in plain text, for the person to read.
 */
export function renderSummary(input: SummaryInput, lane: SetupLane): string {
  const style = styleFor(lane);
  const summary = summaryLines(input, style);
  // In the terminal a blank line first, so the summary stands apart from the answer before it.
  if (lane === "terminal") return lines(["", ...summary]) + renderTips(lane);
  if (lane === "json") return lines(summary) + renderTips(lane);
  return lines([AGENT_SUMMARY_INTRO, "", ...summary, "  Want a few tips for your first session? Just ask.", "", AGENT_TIPS_INTRO, ""]) + renderTips(lane);
}

// ── C6b · Tips ─────────────────────────────────────────────────────────────

const TIP_LINES: readonly string[] = [
  "  When a rule is broken, Rerail can:",
  `    ${"Steer".padEnd(MODE_WIDTH)}${"tell your agent, so it fixes it".padEnd(MODE_TEXT_WIDTH)}most rules start here`,
  `    ${"Log".padEnd(MODE_WIDTH)}only record it, for you to review`,
  "  Change a rule’s mode in the dashboard.",
  "",
  `  ${"/rerail-create".padEnd(COMMAND_WIDTH)}write your own rule, in plain words`,
  `  ${"/rerail-stop".padEnd(COMMAND_WIDTH)}stop Rerail in this repo`,
];

/** C6b (agent, on request) and the tips below T6's divider (terminal dimmed, json plain). */
export function renderTips(lane: SetupLane): string {
  const style = styleFor(lane);
  if (lane === "agent") return lines(["A few things for your first session.", ...TIP_LINES]);
  return lines([`  ${TIPS_DIVIDER}`, ...TIP_LINES].map(style.dim));
}

// ── E1 · The coding agent can't install ────────────────────────────────────

export interface CannotInstallInput {
  /** The line to run in a terminal, e.g. `npx rerail`. */
  readonly installCommand: string;
  /** What blocked the install. */
  readonly reason?: string;
}

const DEFAULT_BLOCKED_REASON = "this session can’t write outside the project folder";

/** E1: the install was blocked before changing anything; hand the person the line to run in a terminal. */
export function renderCannotInstall(input: CannotInstallInput, lane: SetupLane): string {
  const style = styleFor(lane);
  const blocked = [
    `${style.red("✕")} Blocked: ${clean(input.reason ?? DEFAULT_BLOCKED_REASON)}.`,
    "  Rerail installs into your home folder, so it stopped before changing anything.",
  ];
  const command = clean(input.installCommand);
  if (lane === "terminal") return lines([...blocked, `  Run ${command} again from a terminal that can write to your home folder.`]);
  return lines([
    ...blocked,
    "",
    "Tell the person this, as written:",
    "",
    "I couldn’t install Rerail from here. This session is sandboxed, and nothing was changed.",
    "  Run this in your own terminal instead. It takes a few minutes:",
    command,
  ]);
}

// ── E2 · Node is too old ───────────────────────────────────────────────────

export interface NodeTooOldInput {
  readonly found: string;
  readonly requiredMajor: number;
}

/** E2: stop before changing anything, name both versions, and say how to fix it. */
export function renderNodeTooOld(input: NodeTooOldInput, lane: SetupLane): string {
  const style = styleFor(lane);
  return lines([
    `${style.red("✕")} Rerail needs Node ${input.requiredMajor} or newer. You have Node ${clean(input.found)}.`,
    "  Nothing was changed on this machine.",
    `  Upgrade Node, for example:  nvm install ${input.requiredMajor}`,
    "  Then run the same command again.",
  ]);
}

// ── E3 · No repo here ──────────────────────────────────────────────────────

export interface NoRepoInput {
  readonly setupCommand?: string;
}

/** E3: no repo where set-up ran; ask which project to start with. Choosing a folder is the go-ahead to read it. */
export function noRepoQuestion(input: NoRepoInput, lane: SetupLane): QuestionForm {
  const style = styleFor(lane);
  return {
    id: "no-repo",
    lead: [`${style.yellow("△")} No repo found here.  ${style.dim("Rerail needs one project to read.")}`],
    prompt: "Which project do you want to start with?",
    trail: ["Rerail reads it on this computer only, to recommend rule packs."],
    choices: [
      { answer: "folder", label: "Type the folder…", hint: "for example “~/code/acme-web”", args: [], input: folderInput("~/code/acme-web") },
      { answer: "pick-myself", label: "I’ll pick rule packs myself", hint: "Rerail won’t read any code", args: [SETUP_FLAGS.answer, "pick-myself"] },
      { answer: "skip", label: "Skip for now", hint: "start Rerail later, in a session", args: [SETUP_FLAGS.answer, "skip"] },
    ],
    ...(input.setupCommand === undefined ? {} : { setupCommand: input.setupCommand }),
  };
}

export function renderNoRepoQuestion(input: NoRepoInput, lane: SetupLane): string {
  return renderQuestion(noRepoQuestion(input, lane), lane);
}

// ── E4 · "I'll pick rule packs myself" ─────────────────────────────────────

export interface PickedMyselfInput {
  readonly repoName: string;
  readonly connected: readonly CodingAgent[];
  readonly dashboard: DashboardLink;
}

/** E4: Rerail is on without reading any code, and every rule pack is off. */
export function renderPickedMyself(input: PickedMyselfInput, lane: SetupLane): string {
  const style = styleFor(lane);
  const summary = [
    `  ${bullet(`Rerail is on in ${clean(input.repoName)}. No rule packs are on yet.`, style)}`,
    labelled("Connected", connectedText(input.connected), style),
    labelled("Your own", "/rerail-create", style, YOUR_OWN_HINT),
    labelled("Next", "Turn on the rule packs you want.", style, "then open a new session"),
    labelled("Try it", "/rerail-show-me", style, `in a new session: ${SHOW_ME_HINT}`),
    ...linkLines("Manage rules", input.dashboard, style),
    "  Rerail did not read this repo. Changed your mind? `rerail packs recommend` lists the rule packs that fit it.",
  ];
  if (lane === "agent") return lines([AGENT_SUMMARY_INTRO, "", ...summary]);
  if (lane === "json") return lines(summary);
  return lines([bullet("You chose to pick rule packs yourself. Rerail did not read any code.", style), ...summary]);
}

export interface PickPacksInput {
  readonly repoName: string;
  readonly repoPath: string;
  /** Every rule pack set-up fetched, those that fit first when it read the repository. */
  readonly available: readonly AvailablePack[];
  /** Packs set-up could not fetch or check, and why. */
  readonly unavailable: readonly string[];
  /** Overrides `rerail setup` in the agent lane's answer commands. */
  readonly setupCommand?: string;
}

/**
 * E4, before it starts: every rule pack set-up fetched, to pick from. The repository is not read, so a pack picked
 * here is added for the whole repository; "None for now" starts Rerail with none, as E4 did before packs were fetched.
 * With nothing fetched, the question says why and offers only "None for now".
 */
export function pickPacksQuestion(input: PickPacksInput, lane: SetupLane): QuestionForm {
  const style = styleFor(lane);
  const repoFlag = [SETUP_FLAGS.repo, clean(input.repoPath)];
  const available = packList(input.available);
  const none = { answer: "none", label: "None for now", hint: "add rule packs later", args: [SETUP_FLAGS.answer, "pick-myself", ...repoFlag, SETUP_FLAGS.packs, NO_PACKS] };
  return {
    id: "pick-packs",
    lead: [
      input.available.length === 0 ? "  Set-up could not get any of Rerail’s rule packs." : "  Rerail’s rule packs, from npm:",
      ...input.available.map(pack => `    ${withHint(`${clean(pack.id)}@${clean(pack.version)}  ${clean(pack.name)}`, pack.reason === undefined ? undefined : clean(pack.reason), style)}`),
      ...unavailableLines(input.unavailable, style),
    ],
    prompt: `Which rule packs should Rerail start with in ${clean(input.repoName)}?`,
    trail: ["Each one you pick is installed from npm and added for the whole repo. Nothing is installed until you pick."],
    choices: input.available.length === 0 ? [none] : [
      {
        answer: "choose",
        label: "Choose rule packs…",
        args: [SETUP_FLAGS.answer, "pick-myself", ...repoFlag],
        input: { flag: SETUP_FLAGS.packs, prompt: "Which rule packs should Rerail install?", hint: `comma-separated, from: ${available}`, placeholder: "<packs>" },
        agentText: command => `To choose the rule packs: ${command}, with comma-separated packs from: ${available}.`,
      },
      none,
    ],
    ...(input.setupCommand === undefined ? {} : { setupCommand: input.setupCommand }),
  };
}

// ── E5 · Skip for now ──────────────────────────────────────────────────────

export interface SkippedInput {
  readonly connected: readonly CodingAgent[];
}

/** E5: installed and connected, on in no repo; how to start it later. */
export function renderSkipped(input: SkippedInput, lane: SetupLane): string {
  const style = styleFor(lane);
  const summary = [
    `  ${bullet("Rerail is installed. It is not on in any repo yet.", style)}`,
    labelled("Connected", connectedText(input.connected), style),
    "  Next: open a session in one of your repos and type /rerail-start.",
    "  Rerail then sets itself up for that repo: it asks, shows the rule packs that look right, and starts.",
  ];
  return lines(lane === "agent" ? [AGENT_SUMMARY_INTRO, "", ...summary] : summary);
}

// ── Already set up ─────────────────────────────────────────────────────────

export interface StoppedRepoInput {
  readonly repoName: string;
}

/** Set-up ran again in a repo where Rerail was set up and then stopped: it changes nothing there. */
export function renderStoppedRepo(input: StoppedRepoInput, lane: SetupLane): string {
  const style = styleFor(lane);
  const start = lane === "terminal" ? "rerail start" : "/rerail-start";
  return lines([`  ${style.dim("○")} Rerail is set up in ${clean(input.repoName)}, and stopped there. Start it again with ${start}.`]);
}

// ── Terminal prompts ───────────────────────────────────────────────────────

/** Under a question in the terminal: Enter takes the answer the cursor is on. */
export function renderAnswerPrompt(): string {
  return `\n${TERMINAL_STYLE.dim("Enter for 1, or type a number:")} `;
}

/** After "Use a different folder…" or "Type the folder…". */
export function renderFolderPrompt(): string {
  return `  ${TERMINAL_STYLE.bold("Which folder?")} `;
}

export interface FolderNotFoundInput {
  readonly folder: string;
}

/** The folder named at set-up is not there; nothing was read. */
export function renderFolderNotFound(input: FolderNotFoundInput, lane: SetupLane): string {
  const style = styleFor(lane);
  return lines([`${style.yellow("△")} There is no folder at ${clean(input.folder)}. Nothing was read.`]);
}

export interface PacksPromptInput {
  readonly available: readonly AvailablePack[];
}

/** After "Choose rule packs…": every rule pack set-up fetched, numbered, then which to install. */
export function renderPacksPrompt(input: PacksPromptInput): string {
  const style = TERMINAL_STYLE;
  const rows = input.available.map((pack, index) =>
    `  ${index + 1}. ${clean(pack.id)}@${clean(pack.version)}  ${withHint(clean(pack.name), pack.reason === undefined ? undefined : clean(pack.reason), style)}`);
  return [...rows, `  ${style.bold("Which rule packs should Rerail install?")} ${style.dim("numbers or ids, comma-separated:")} `].join("\n");
}
