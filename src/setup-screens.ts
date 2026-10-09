import type { SetupOption, SetupOptionInput, SetupQuestion, SetupQuestionId } from "./setup-step.js";
import { RERAIL_BRAND_FOREGROUND } from "./terminal-brand.generated.js";

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
  readonly name: string;
  readonly ruleCount: number;
  readonly reason: string;
  /** How many of its rules start in Log rather than Steer; none when absent. */
  readonly logRuleCount?: number;
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
export const SETUP_FLAGS = { answer: "--answer", repo: "--repo", agents: "--agents", relay: "--relay" } as const;

/** Answer keys in the order each question numbers its answers (answer 1 is index 0). */
export const READ_REPO_ANSWERS = ["yes", "other-folder", "pick-myself"] as const;
export const CONFIRM_PACKS_ANSWERS = ["start", "review", "other", "pick-myself"] as const;
export const NO_REPO_ANSWERS = ["folder", "pick-myself", "skip"] as const;
export type ReadRepoAnswer = (typeof READ_REPO_ANSWERS)[number];
export type ConfirmPacksAnswer = (typeof CONFIRM_PACKS_ANSWERS)[number];
export type NoRepoAnswer = (typeof NO_REPO_ANSWERS)[number];

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
const clean = (value: string): string => value.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu, "");

const MASCOT_HAPPY = ">^ᴥ^<";
const LABEL_WIDTH = 15;
const COMMAND_WIDTH = 19;
const TABLE_RULES_WIDTH = 7;
const TABLE_MIN_PACK_WIDTH = 16;
const TIPS_DIVIDER = "── Good to know · optional ──────────────────────────────────";

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
/** One shell word; set-up quotes the launcher path it hands the coding agent with it too. */
export function shellQuote(value: string): string {
  return SHELL_SAFE.test(value) ? value : `'${value.replaceAll("'", "'\\''")}'`;
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
  const command = [form.setupCommand ?? SETUP_COMMAND, ...choice.args.map(shellQuote), ...input].join(" ");
  return choice.agentText === undefined ? command : choice.agentText(command);
}

/**
 * Terminal: the body, then the answers with a cursor on the first.
 * Agent: the body and answers as text to relay, then one command per answer.
 * Json: the body without the question line, which `setupQuestion` carries with the answers.
 */
export function renderQuestion(form: QuestionForm, lane: SetupLane): string {
  const style = styleFor(lane);
  if (lane === "json") return lines([...form.lead, ...form.trail]);
  const body = [...form.lead, style.bold(form.prompt), ...form.trail];
  const answer = (choice: Choice, index: number): string => withHint(`${index + 1}. ${choice.label}`, choice.hint, style);
  if (lane === "terminal") {
    return lines([...body, ...form.choices.map((choice, index) => `${index === 0 ? style.brand("❯") : " "} ${answer(choice, index)}`)]);
  }
  return lines([
    AGENT_QUESTION_INTRO,
    "",
    ...body,
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

// ── T2 · Installed ─────────────────────────────────────────────────────────

export interface InstalledInput {
  readonly version: string;
}

/** T2's finished line (terminal); the line C3 opens with (agent, json). */
export function renderInstalled(input: InstalledInput, lane: SetupLane): string {
  if (lane !== "terminal") return lines(["Rerail is installed. Nothing is on yet."]);
  const style = styleFor(lane);
  return lines([`${style.brand(MASCOT_HAPPY)}   rerail ${clean(input.version)} installed  ${style.dim("nothing is on yet")}`]);
}

// ── T3 / C3 · Ask before reading ───────────────────────────────────────────

export interface ReadRepoInput {
  readonly repoName: string;
  readonly repoPath: string;
  /** Overrides `rerail setup` in the agent lane's answer commands. */
  readonly setupCommand?: string;
}

/** T3 / C3: Rerail names the repo, says what it will do, and asks before reading it. */
export function readRepoQuestion(input: ReadRepoInput): QuestionForm {
  const repo = clean(input.repoName);
  const repoFlag = [SETUP_FLAGS.repo, clean(input.repoPath)];
  return {
    id: "read-repo",
    lead: [],
    prompt: `You’re working in ${repo}. Can Rerail read it to pick the rule packs that look right?`,
    trail: [
      "It matches the files in this repo against the rule packs that come with Rerail.",
      "It reads on this computer only. Nothing leaves it. Nothing is turned on yet.",
    ],
    choices: [
      { answer: "yes", label: `Yes, read ${repo}`, args: [SETUP_FLAGS.answer, "yes", ...repoFlag] },
      { answer: "other-folder", label: "Use a different folder…", hint: "for example “~/code/landing-page”", args: [], input: folderInput("~/code/landing-page") },
      { answer: "pick-myself", label: "I’ll pick rule packs myself", hint: "Rerail won’t read the repo", args: [SETUP_FLAGS.answer, "pick-myself", ...repoFlag] },
    ],
    ...(input.setupCommand === undefined ? {} : { setupCommand: input.setupCommand }),
  };
}

export function renderReadRepoQuestion(input: ReadRepoInput, lane: SetupLane): string {
  return renderQuestion(readRepoQuestion(input), lane);
}

// ── T4 / C4 · Recommended rule packs and the confirm question ──────────────

export interface RecommendedPacksInput {
  readonly repoName: string;
  readonly repoPath: string;
  readonly packs: readonly RecommendedPack[];
  readonly agentsFound: readonly CodingAgent[];
  readonly agentsNotFound: readonly CodingAgent[];
  readonly setupCommand?: string;
}

function packTable(packs: readonly RecommendedPack[], style: Style): string[] {
  const names = packs.map(pack => clean(pack.name));
  const nameWidth = Math.max(TABLE_MIN_PACK_WIDTH, ...names.map(name => name.length + 2));
  const header = style.dim(`  ${"Rule pack".padEnd(nameWidth)}${"Rules".padEnd(TABLE_RULES_WIDTH)}Why`);
  const rows = packs.map(
    (pack, index) => `  ${(names[index] ?? "").padEnd(nameWidth)}${String(pack.ruleCount).padEnd(TABLE_RULES_WIDTH)}${clean(pack.reason)}`,
  );
  return [header, ...rows];
}

function recommendationLines(input: RecommendedPacksInput, style: Style): string[] {
  const repo = clean(input.repoName);
  const count = input.packs.length;
  const found = input.agentsFound.length === 0 ? "no coding agents found" : agentNames(input.agentsFound).join(", ");
  const notFound = input.agentsNotFound.length === 0 ? undefined : `not found: ${agentNames(input.agentsNotFound).join(", ")}`;
  const intro =
    count === 0
      ? `  None of the rule packs that come with Rerail look right for ${repo}.`
      : `  Of the rule packs that come with Rerail, ${count} ${count === 1 ? "looks" : "look"} right for ${repo}.`;
  return [
    intro,
    ...(count === 0 ? [] : packTable(input.packs, style)),
    labelled("Connected to", found, style, notFound),
    ...(count === 0 ? [] : [startModeLine(input.packs)]),
    "  More rule packs are in the dashboard, off until you turn them on.",
  ];
}

/** How the recommended rules start, from what each rule pack says: Steer, unless some start in Log. */
function startModeLine(packs: readonly RecommendedPack[]): string {
  const logged = packs.reduce((total, pack) => total + (pack.logRuleCount ?? 0), 0);
  const steer = "your coding agent is told as it works, and fixes it.";
  if (logged === 0) return `  Every rule starts in Steer: ${steer}`;
  return `  Rules start in Steer: ${steer} ${logged === 1 ? "1 starts" : `${logged} start`} in Log, recorded only.`;
}

function confirmQuestion(input: RecommendedPacksInput, prompt: string, lead: readonly string[]): QuestionForm {
  const repoFlag = [SETUP_FLAGS.repo, clean(input.repoPath)];
  const ids = input.agentsFound.map(agent => `${clean(agent.id)} (${clean(agent.name)})`).join(", ");
  const idList = ids === "" ? "none found" : ids;
  return {
    id: "confirm-packs",
    lead,
    prompt,
    trail: ["Nothing is on until you say yes."],
    choices: [
      { answer: "start", label: "Yes, start Rerail", args: [SETUP_FLAGS.answer, "start", ...repoFlag] },
      { answer: "review", label: "Review them first", hint: "opens the dashboard", args: [SETUP_FLAGS.answer, "review", ...repoFlag] },
      {
        answer: "other",
        label: "Type something else…",
        hint: "only some coding agents, by id",
        // Only some coding agents is what set-up can do with an answer of their own.
        args: [SETUP_FLAGS.answer, "start", ...repoFlag],
        input: { flag: SETUP_FLAGS.agents, prompt: "Which coding agents should Rerail connect to?", hint: `comma-separated ids from: ${idList}`, placeholder: "<ids>" },
        agentText: command =>
          `To start with only some coding agents: ${command}, with comma-separated ids from: ${idList}. For anything else, say what set-up can do and ask again.`,
      },
      { answer: "pick-myself", label: "I’ll pick rule packs myself", args: [SETUP_FLAGS.answer, "pick-myself", ...repoFlag] },
    ],
    ...(input.setupCommand === undefined ? {} : { setupCommand: input.setupCommand }),
  };
}

function startQuestion(input: RecommendedPacksInput): string {
  const repo = clean(input.repoName);
  const count = input.packs.length;
  if (count === 0) return `Start Rerail in ${repo} with no rule packs on?`;
  return `Start Rerail in ${repo} with ${count === 1 ? "this rule pack" : `these ${count} rule packs`}?`;
}

/** T4 / C4: the rule packs that look right, each with its rule count and reason, then the confirm question. */
export function recommendedPacksQuestion(input: RecommendedPacksInput, lane: SetupLane): QuestionForm {
  const style = styleFor(lane);
  const echo = lane === "terminal" ? [bullet(`You said yes. Rerail read ${clean(input.repoName)}.`, style)] : [];
  return confirmQuestion(input, startQuestion(input), [...echo, ...recommendationLines(input, style)]);
}

export function renderRecommendedPacks(input: RecommendedPacksInput, lane: SetupLane): string {
  return renderQuestion(recommendedPacksQuestion(input, lane), lane);
}

// ── WR · Review in the dashboard, then confirm back here ───────────────────

export interface PackReviewInput extends RecommendedPacksInput {
  /** The dashboard's review page, e.g. `http://127.0.0.1:7777/#review`. */
  readonly review: DashboardLink;
}

/**
 * After "Review them first": the review link, then the confirm question again.
 * Set-up does not wait on the browser; the person confirms back here.
 */
export function packReviewQuestion(input: PackReviewInput, lane: SetupLane): QuestionForm {
  const style = styleFor(lane);
  const lead = [
    bullet("Review the rule packs in the dashboard. Switch off any you don’t want, then come back here to start Rerail.", style),
    ...linkLines("Review", input.review, style),
  ];
  return confirmQuestion(input, `Start Rerail in ${clean(input.repoName)} with the rule packs you kept on?`, lead);
}

export function renderPackReview(input: PackReviewInput, lane: SetupLane): string {
  return renderQuestion(packReviewQuestion(input, lane), lane);
}

// ── T5 / C5 · Started ──────────────────────────────────────────────────────

export interface StartedInput {
  readonly repoName: string;
  readonly packCount: number;
  readonly connected: readonly CodingAgent[];
}

/** T5 / C5: Rerail is on in this repo, and which coding agents it is connected to. */
export function renderStarted(input: StartedInput, lane: SetupLane): string {
  const style = styleFor(lane);
  const repo = clean(input.repoName);
  const connected = connectedText(input.connected);
  if (lane !== "terminal") return lines([`Rerail is on in ${repo}. Connected to ${connected}.`]);
  return lines([
    bullet(`Confirmed: ${plural(input.packCount, "rule pack")} · ${connected}`, style),
    `${style.brand(MASCOT_HAPPY)}   Rerail is on in ${repo}.  ${style.dim(`connected to ${connected}`)}`,
    style.dim("Enter to see the summary."),
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

export interface SummaryInput {
  readonly repoName: string;
  /** The rule packs now on, by name. Never the rules themselves. */
  readonly packNames: readonly string[];
  readonly ruleCount: number;
  readonly connected: readonly CodingAgent[];
  readonly dashboard: DashboardLink;
  /** Other repos where Rerail is already started; empty on a first set-up. */
  readonly otherStartedRepos?: readonly string[];
}

function otherReposLine(names: readonly string[], style: Style): string {
  if (names.length === 0) {
    return labelled("Other repos", "Rerail has not been started there", style, "type /rerail-start there to set one up");
  }
  return labelled("Other repos", `Rerail is also on in ${andList(names.map(clean))}`, style, "type /rerail-start in another repo to set it up");
}

function summaryLines(input: SummaryInput, style: Style): string[] {
  const packCount = input.packNames.length;
  // No rule pack fit the repository: say so plainly, and where to add one.
  const packsOn = packCount === 0
    ? "No rule packs are on yet."
    : `${plural(packCount, "rule pack")} ${packCount === 1 ? "is" : "are"} on, with ${plural(input.ruleCount, "rule")}.`;
  return [
    `  ${bullet(`Rerail is on in ${clean(input.repoName)}. ${packsOn}`, style)}`,
    packCount === 0
      ? labelled("Rule packs", "none yet", style, "add one in the dashboard if it suits this repo")
      : labelled("Rule packs", input.packNames.map(clean).join(" · "), style),
    labelled("Your own", "/rerail-create", style, YOUR_OWN_HINT),
    otherReposLine(input.otherStartedRepos ?? [], style),
    labelled("Connected", connectedText(input.connected), style),
    labelled("Next", "Open a new session when you’re ready.", style, "Rerail starts there"),
    ...linkLines("Manage rules", input.dashboard, style),
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
  if (lane !== "agent") return lines(summary) + renderTips(lane);
  return lines([AGENT_SUMMARY_INTRO, "", ...summary, "  Want a few tips for your first session? Just ask.", "", AGENT_TIPS_INTRO, ""]) + renderTips(lane);
}

// ── C6b · Tips ─────────────────────────────────────────────────────────────

const TIP_LINES: readonly string[] = [
  "  Three things can happen when a rule is broken.",
  `  ${"Log".padEnd(LABEL_WIDTH)}Rerail records the violation for you. Your coding agent isn’t told.`,
  `  ${"Steer".padEnd(LABEL_WIDTH)}Your coding agent is told as it works, and fixes it. It isn’t stopped.`,
  `  ${"Block".padEnd(LABEL_WIDTH)}Your coding agent is stopped before it acts.  not in this version`,
  "  Each rule starts in Steer or Log, as its rule pack says. See each rule in the dashboard.",
  `  ${"/rerail".padEnd(COMMAND_WIDTH)}see what Rerail is doing, and every command`,
  `  ${"/rerail-create".padEnd(COMMAND_WIDTH)}create a rule, in your own words`,
  `  ${"/rerail-stop".padEnd(COMMAND_WIDTH)}stop Rerail in this repo, straight away`,
  "  Rerail never edits your instruction files.",
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
    ...linkLines("Manage rules", input.dashboard, style),
    "  Rerail did not read this repo. Changed your mind? Ask it to recommend rule packs, in the dashboard.",
  ];
  if (lane === "agent") return lines([AGENT_SUMMARY_INTRO, "", ...summary]);
  if (lane === "json") return lines(summary);
  return lines([bullet("You chose to pick rule packs yourself. Rerail did not read any code.", style), ...summary]);
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
  return `  ${TERMINAL_STYLE.dim("Type a number, or press Enter for 1:")} `;
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

export interface AgentsPromptInput {
  readonly found: readonly CodingAgent[];
}

/** After "Type something else…": which coding agents to connect. */
export function renderAgentsPrompt(input: AgentsPromptInput): string {
  const example = input.found.length === 0 ? "claude-code, cursor" : input.found.map(agent => clean(agent.id)).join(", ");
  return `  ${TERMINAL_STYLE.bold("Which coding agents should Rerail connect to?")} ${TERMINAL_STYLE.dim(`for example “${example}”:`)} `;
}
