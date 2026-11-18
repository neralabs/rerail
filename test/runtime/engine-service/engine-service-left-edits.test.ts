import { writeFileSync } from "node:fs";
import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { diffLines } from "diff";
import { describe, it } from "vitest";
import { type AdapterEvent, type CodeRuleAuditor, leaveUntaken } from "../../../src/engine/index.js";
import { engineRequestBody, ingestEngineEvent, startEngineService } from "../../../src/runtime/engine-service.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";

/*
 * A property of the tool calls a session's daemon checks after they ran (their hooks left them while it did not read):
 * whatever ran around them, each line a rule flags that the check of a call seen live would tell the agent is told, and
 * a call read against a rebuilt "before" is told exactly those lines. Generated sequences of Edit, MultiEdit and Write
 * calls, seen live or left, and another program's writes, over a few files; each is replayed through an engine service
 * whose rules flag every line holding `print(` and every new file named like `n1.py`.
 */

type EditBatch = Parameters<CodeRuleAuditor["auditGeneratedEdits"]>[1];
type BatchEdit = EditBatch["edits"][number];

const SESSION = "left-edits";
/** Off Windows the checkout sits here: a Unix socket path must fit 104 bytes on macOS and 108 on Linux. */
const SOCKET_ROOT = "/tmp";
/** The file rule: it flags a file named so, with no span, which a call that creates it generated. */
const FILE_RULE = /^n\d+\.py$/u;
const SEQUENCES = 500;
const CHUNK = 25;
const FIRST_SEED = 7_100;

interface TextChange {
  old_string: string;
  new_string: string;
  replace_all: boolean;
}
type Actor = "live" | "left" | "foreign";
interface Step {
  actor: Actor;
  path: string;
  tool: "Edit" | "MultiEdit" | "Write";
  edits?: TextChange[];
  content?: string;
}
interface Sequence {
  seed: number;
  /** The session first sees its folder after the calls left wrote it (a spell right after its start). */
  cold: boolean;
  files: Record<string, string>;
  before: Step[];
  hold: Step[];
}

// ---- texts and the rules ----

function linesOf(text: string): string[] {
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

/** The indexes of the lines of `after` that `diffLines` counts as added, as the enforcer measures an edit's lines. */
function addedLines(before: string, after: string): number[] {
  const added: number[] = [];
  let index = 0;
  for (const change of diffLines(before, after)) {
    const count = linesOf(change.value).length;
    if (change.added) for (let k = 0; k < count; k += 1) added.push(index + k);
    if (!change.removed) index += count;
  }
  return added;
}

/** What the rules tell for one edit of a batch, as `#selectViolations` selects: its lines, or the whole file. */
function toldFor(edit: BatchEdit): string[] {
  if (edit.generatedText === undefined) return [];
  const whole = edit.previousReadError !== undefined || edit.previousText === undefined;
  const lines = linesOf(edit.generatedText);
  const selected = whole ? lines.map((_, index) => index) : addedLines(edit.previousText!, edit.generatedText);
  const prints = selected.filter(index => lines[index]!.includes("print(")).map(index => `print in ${edit.path}:${index + 1}`);
  return whole && FILE_RULE.test(edit.path) ? [...prints, `file ${edit.path}`] : prints;
}

/** An edit read against a rebuilt "before": the call captured none, and its edit carries a text it was made on. */
function readExact(edit: BatchEdit): boolean {
  return edit.previousText !== undefined && edit.previousReadError === undefined;
}

// ---- the tool's edits, as Claude Code makes them ----

function appliedOne(text: string, change: TextChange): string | undefined {
  if (change.old_string === "" || change.old_string === change.new_string) return undefined;
  const places = text.split(change.old_string).length - 1;
  if (places === 0 || (!change.replace_all && places > 1)) return undefined;
  if (change.replace_all) return text.split(change.old_string).join(change.new_string);
  return text.replace(change.old_string, () => change.new_string);
}

/** The text an Edit or MultiEdit leaves, or undefined where the tool refuses it; an empty old text creates a file. */
function applied(text: string | undefined, edits: readonly TextChange[]): string | undefined {
  if (edits[0]?.old_string === "") return text === undefined && edits.length === 1 ? edits[0].new_string : undefined;
  if (text === undefined) return undefined;
  return edits.reduce<string | undefined>((current, change) => (current === undefined ? undefined : appliedOne(current, change)), text);
}

function stepText(text: string | undefined, step: Step): string | undefined {
  return step.tool === "Write" ? step.content : applied(text, step.edits ?? []);
}

// ---- the model: each line of each file with the id of the step that wrote it ----

interface Line {
  id: number;
  text: string;
}

class Model {
  readonly files = new Map<string, Line[]>();
  #next = 1;

  constructor(files: Record<string, string>) {
    for (const [path, text] of Object.entries(files)) this.files.set(path, linesOf(text).map(line => this.#line(line)));
  }

  text(path: string): string | undefined {
    const lines = this.files.get(path);
    return lines === undefined ? undefined : lines.map(line => `${line.text}\n`).join("");
  }

  /** Moves `path` to `after`, keeping the ids of the lines `diffLines` keeps; returns the lines it adds. */
  move(path: string, after: string): Line[] {
    const before = this.files.get(path) ?? [];
    const lines: Line[] = [];
    const added: Line[] = [];
    let at = 0;
    for (const change of diffLines(this.text(path) ?? "", after)) {
      const texts = linesOf(change.value);
      if (change.added) {
        const fresh = texts.map(text => this.#line(text));
        added.push(...fresh);
        lines.push(...fresh);
        continue;
      }
      if (!change.removed) lines.push(...before.slice(at, at + texts.length));
      at += texts.length;
    }
    this.files.set(path, lines);
    return added;
  }

  #line(text: string): Line {
    return { id: this.#next++, text };
  }
}

/**
 * What a check seen live tells for a call that moved `path` from `before` to what it left (`added`): the lines it
 * added that a rule flags, and the file itself where it created a file the file rule names.
 */
function toldLive(path: string, created: boolean, added: readonly Line[]): Set<string> {
  const told = new Set(added.filter(line => line.text.includes("print(")).map(line => `${path}#${line.id}`));
  if (created && FILE_RULE.test(path)) told.add(`file ${path}`);
  return told;
}

/** `told` (the steers' texts) as the lines of the files as they end, by id. */
function toldIds(model: Model, told: readonly string[]): Set<string> {
  const ids = new Set<string>();
  for (const steer of told) {
    const match = /print in (.+):(\d+)$/u.exec(steer);
    const line = match === null ? undefined : model.files.get(match[1]!)?.[Number(match[2]) - 1];
    if (match !== null && line !== undefined) ids.add(`${match[1]}#${line.id}`);
    const file = /file (.+)$/u.exec(steer);
    if (file !== null) ids.add(`file ${file[1]}`);
  }
  return ids;
}

/** The ids of `told` still in the files as they end: a line a later write removed is no one's to tell. */
function stillThere(model: Model, told: ReadonlySet<string>): Set<string> {
  const present = new Set<string>();
  for (const [path, lines] of model.files) {
    for (const line of lines) present.add(`${path}#${line.id}`);
    present.add(`file ${path}`);
  }
  return new Set([...told].filter(id => present.has(id)));
}

// ---- generated sequences ----

/** A seeded generator of numbers in [0, 1) (mulberry32). */
function seeded(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let mixed = Math.imul(state ^ (state >>> 15), 1 | state);
    mixed = (mixed + Math.imul(mixed ^ (mixed >>> 7), 61 | mixed)) ^ mixed;
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4_294_967_296;
  };
}

class Draw {
  #fresh = 100;
  constructor(readonly next: () => number) {}

  below(count: number): number {
    return Math.floor(this.next() * count);
  }

  chance(odds: number): boolean {
    return this.next() < odds;
  }

  pick<T>(items: readonly T[]): T {
    return items[this.below(items.length)]!;
  }

  /** A line's text: a print (one a file may hold already, now and then), an assignment, or `pass`. */
  content(): string {
    const number = this.chance(0.3) ? 1 + this.below(3) : this.#fresh++;
    const kind = this.below(10);
    if (kind < 5) return `    print(${number})`;
    return kind < 8 ? `    x = (${number})` : "    pass";
  }
}

function freshFile(draw: Draw, name: string): string {
  const body = Array.from({ length: 1 + draw.below(3) }, () => `${draw.content()}\n`).join("");
  return `def ${name}():\n${body}    return 0\n`;
}

/** The smallest block of lines from `start` (1 to 3) that `text` holds once, joined with or without its last newline. */
function uniqueBlock(text: string, lines: readonly string[], start: number, newline: boolean): string[] | undefined {
  for (let size = 1; size <= 3 && start + size <= lines.length; size += 1) {
    const block = lines.slice(start, start + size);
    if (text.split(block.join("\n") + (newline ? "\n" : "")).length === 2) return block;
  }
  return undefined;
}

function joined(lines: readonly string[], newline: boolean): string {
  return lines.join("\n") + (newline ? "\n" : "");
}

/** One change an Edit asks for in `text`: a line added, changed or removed, or one line's text changed everywhere. */
function changeIn(draw: Draw, text: string): TextChange | undefined {
  const lines = linesOf(text);
  if (lines.length < 2) return undefined;
  const start = 1 + draw.below(lines.length - 1);
  const kind = draw.below(10);
  const newline = kind === 7 || draw.chance(0.5);
  if (kind >= 8) return everywhereChange(draw, lines[start]!, newline);
  const block = uniqueBlock(text, lines, start, newline);
  return block === undefined ? undefined : blockChange(draw, block, kind, newline);
}

function everywhereChange(draw: Draw, line: string, newline: boolean): TextChange {
  const end = newline ? "\n" : "";
  return { old_string: line + end, new_string: draw.content() + end, replace_all: true };
}

function blockChange(draw: Draw, block: readonly string[], kind: number, newline: boolean): TextChange {
  const at = draw.below(block.length + 1);
  const changed = [...block];
  if (kind < 4) changed.splice(at, 0, draw.content());
  else if (kind < 7) changed.splice(Math.min(at, block.length - 1), 1, draw.content());
  else changed.splice(Math.min(at, block.length - 1), 1);
  return { old_string: joined(block, newline), new_string: changed.length === 0 ? "" : joined(changed, newline), replace_all: false };
}

/** A step of `actor` on one of `paths` (or a new file), drawn against the texts as they stand; undefined if none fits. */
function drawStep(draw: Draw, actor: Actor, texts: Map<string, string>, paths: readonly string[]): Step | undefined {
  const kind = draw.below(10);
  if (actor === "foreign") return editStep(draw, actor, texts, draw.pick(paths), 1);
  if (kind === 9) return createStep(draw, actor, texts);
  const path = draw.pick(paths);
  if (kind >= 7) return writeStepOf(draw, actor, path, texts.get(path)!);
  return editStep(draw, actor, texts, path, kind >= 5 ? 2 : 1);
}

function editStep(draw: Draw, actor: Actor, texts: Map<string, string>, path: string, count: number): Step | undefined {
  const edits = drawEdits(draw, texts.get(path)!, count);
  return edits === undefined ? undefined : { actor, path, tool: count === 2 ? "MultiEdit" : "Edit", edits };
}

/** A Write of `path` whose content is its text with one change made, as an agent rewriting the file leaves it. */
function writeStepOf(draw: Draw, actor: Actor, path: string, text: string): Step | undefined {
  const change = changeIn(draw, text);
  const content = change === undefined ? undefined : applied(text, [change]);
  return content === undefined ? undefined : { actor, path, tool: "Write", content };
}

function drawEdits(draw: Draw, text: string, count: number): TextChange[] | undefined {
  const edits: TextChange[] = [];
  let current: string | undefined = text;
  for (let k = 0; k < count && current !== undefined; k += 1) {
    const change = changeIn(draw, current);
    current = change === undefined ? undefined : appliedOne(current, change);
    if (change !== undefined && current !== undefined) edits.push(change);
  }
  return current === undefined ? undefined : edits;
}

function createStep(draw: Draw, actor: Actor, texts: Map<string, string>): Step {
  const path = `n${texts.size}.py`;
  return { actor, path, tool: "Edit", edits: [{ old_string: "", new_string: freshFile(draw, "n"), replace_all: false }] };
}

/** `count` steps of the actors `actors` draws, each applied to `texts` as drawn; a step that cannot be made is skipped. */
function drawSteps(draw: Draw, texts: Map<string, string>, count: number, actors: () => Actor): Step[] {
  const steps: Step[] = [];
  const used: string[] = [];
  for (let k = 0; k < count; k += 1) {
    const paths = used.length > 0 && draw.chance(0.5) ? used : [...texts.keys()];
    const step = drawStep(draw, actors(), texts, paths);
    const after = step === undefined ? undefined : stepText(texts.get(step.path), step);
    if (step === undefined || after === undefined) continue;
    texts.set(step.path, after);
    used.push(step.path);
    steps.push(step);
  }
  return steps;
}

function sequenceOf(seed: number): Sequence {
  const draw = new Draw(seeded(seed));
  const files = { "f0.py": freshFile(draw, "f0"), "f1.py": freshFile(draw, "f1") };
  const texts = new Map(Object.entries(files));
  const cold = draw.chance(0.25);
  const before = cold ? [] : drawSteps(draw, texts, draw.below(4), () => (draw.chance(0.6) ? "live" : "foreign"));
  const hold = drawSteps(draw, texts, 1 + draw.below(6), () => (draw.chance(0.75) ? "left" : "foreign"));
  return { seed, cold, files, before, hold };
}

// ---- one sequence through an engine service ----

interface LeftCall {
  step: Step;
  index: number;
  callAt: number;
  resultAt: number;
  /** What a check seen live would tell for it (`toldLive`). */
  live: Set<string>;
}

/**
 * An engine service over a checkout of its own holding `files`, which checks the events hooks leave in its home, with a
 * Claude Code session started: its auditor keeps each edit batch and steers once for each finding `toldFor` gives, each
 * steer naming its batch so none stands for another. Where `cold`, its evaluator opens only at `open`.
 */
async function serviceFor(files: Record<string, string>, cold: boolean) {
  const root = await realpath(await mkdtemp(join(process.platform === "win32" ? tmpdir() : SOCKET_ROOT, "rerail-left-")));
  for (const [path, text] of Object.entries(files)) writeFileSync(join(root, path), text);
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const socketPath = testEngineEndpoint(root);
  const opening = Promise.withResolvers<void>();
  if (!cold) opening.resolve();
  const opened = Promise.withResolvers<void>();
  const walking = Promise.withResolvers<void>();
  const batches: EditBatch[] = [];
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    untakenEvents: { home: root, lateChecked: () => undefined },
    createAuditor: ({ steerDelivery }) => ({
      openWorkspace: async () => {
        opened.resolve();
        await opening.promise;
      },
      acceptsPath: () => {
        walking.resolve();
        return true;
      },
      auditGeneratedEdits: async (_sessionId, batch) => {
        batches.push(batch);
        const told = batch.mode === "log" ? [] : batch.edits.flatMap(toldFor);
        for (const finding of told) await steerDelivery.requestSteer(SESSION, "steer", `[${batch.revision}] ${finding}`, { revision: batch.revision, scope: [] });
        return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
      },
      settleGeneratedEdits: async () => undefined,
      close: async () => undefined,
    }),
  });
  await service.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: SESSION, cwd: root } });
  await opened.promise;
  const send = async (event: AdapterEvent) => (await ingestEngineEvent({ socketPath }, event)).map(delivery => delivery.content);
  const view = async () => {
    opening.resolve();
    await walking.promise;
    await send({ kind: "turn-end", eventId: "view", sessionId: SESSION, text: "" });
  };
  const leave = async (event: AdapterEvent, arrivedAt: number) => await leaveUntaken(root, process.pid, engineRequestBody(event), arrivedAt);
  const close = async () => {
    await service.close();
    await removeFolder(root);
  };
  return { root, service, batches, send, view, leave, close };
}

type Service = Awaited<ReturnType<typeof serviceFor>>;

function inputOf(step: Step): Record<string, unknown> {
  if (step.tool === "Write") return { file_path: step.path, content: step.content };
  if (step.tool === "MultiEdit") return { file_path: step.path, edits: step.edits };
  return { file_path: step.path, ...step.edits![0] };
}

/** Writes what `step` leaves, moving the model; undefined where the step no longer fits (a shrunk sequence). */
function writeStep(service: Service, model: Model, step: Step): Line[] | undefined {
  const before = model.text(step.path);
  const after = stepText(before, step);
  if (after === undefined) return undefined;
  writeFileSync(join(service.root, step.path), after);
  return model.move(step.path, after);
}

async function liveStep(service: Service, model: Model, step: Step, index: number): Promise<void> {
  if (step.actor === "foreign") {
    writeStep(service, model, step);
    return;
  }
  if (stepText(model.text(step.path), step) === undefined) return;
  const tool = { sessionId: SESSION, toolCallId: `live-${index}`, toolName: step.tool } as const;
  await service.send({ kind: "call", eventId: `live-${index}:call`, ...tool, input: inputOf(step) });
  writeStep(service, model, step);
  await service.send({ kind: "result", eventId: `live-${index}:result`, ...tool, output: "ok" });
}

/** When each hook met its event: now, and after every earlier one, so the events keep the order they happened in. */
class Ticks {
  #last = 0;
  next(): number {
    this.#last = Math.max(Date.now(), this.#last + 1);
    return this.#last;
  }
}

/** A step of the hold: another program's write, or a call whose tool writes while its hooks leave its events. */
function holdStep(service: Service, model: Model, step: Step, index: number, ticks: Ticks): LeftCall | undefined {
  const created = model.text(step.path) === undefined;
  const callAt = ticks.next();
  const added = writeStep(service, model, step);
  if (added === undefined || step.actor === "foreign") return undefined;
  return { step, index, callAt, resultAt: ticks.next(), live: toldLive(step.path, created, added) };
}

async function leaveCall(service: Service, call: LeftCall): Promise<void> {
  const tool = { sessionId: SESSION, toolCallId: `left-${call.index}`, toolName: call.step.tool } as const;
  await service.leave({ kind: "call", eventId: `left-${call.index}:call`, ...tool, input: inputOf(call.step) }, call.callAt);
  await service.leave({ kind: "result", eventId: `left-${call.index}:result`, ...tool, output: "ok" }, call.resultAt);
}

/** The steers the session's next answers carry, until one carries none. */
async function toldNext(service: Service): Promise<string[]> {
  const told: string[] = [];
  for (let k = 0; k < 5; k += 1) {
    const answer = await service.send({ kind: "user-prompt", eventId: `next-${k}`, sessionId: SESSION, text: "next" });
    told.push(...answer);
    if (answer.length === 0) break;
  }
  return told;
}

const listed = (ids: ReadonlySet<string>): string => [...ids].join(", ") || "nothing";

/** What breaks the property in one left call's check, as lines for the report. */
function callProblems(model: Model, call: LeftCall, edit: BatchEdit | undefined, told: ReadonlySet<string>): string[] {
  const wanted = stillThere(model, call.live);
  const name = `left call ${call.index} (${call.step.tool} of ${call.step.path})`;
  if (edit === undefined || edit.path !== call.step.path) return [`${name}: no check of it read the file`];
  const problems = [...wanted].filter(id => !told.has(id)).map(id => `${name}: live it tells ${id}, here no answer does`);
  return readExact(edit) ? [...problems, ...exactProblems(name, toldIds(model, toldFor(edit)), wanted)] : problems;
}

/** A call read as exact must tell what the live check tells, no line more and none fewer. */
function exactProblems(name: string, exact: ReadonlySet<string>, wanted: ReadonlySet<string>): string[] {
  const same = exact.size === wanted.size && [...exact].every(id => wanted.has(id));
  return same ? [] : [`${name}: read as exact it tells ${listed(exact)}, live it tells ${listed(wanted)}`];
}

/** Replays `sequence` and returns what breaks the property; none when it holds. */
async function problemsOf(sequence: Sequence): Promise<string[]> {
  const service = await serviceFor(sequence.files, sequence.cold);
  try {
    const model = new Model(sequence.files);
    if (!sequence.cold) await service.view();
    for (const [index, step] of sequence.before.entries()) await liveStep(service, model, step, index);
    const ticks = new Ticks();
    const left = sequence.hold.map((step, index) => holdStep(service, model, step, index, ticks)).filter(call => call !== undefined);
    if (sequence.cold) await service.view();
    const mark = service.batches.length;
    for (const call of left) await leaveCall(service, call);
    await service.service.untaken.taken();
    const answers = await toldNext(service);
    const told = toldIds(model, answers);
    const checks = service.batches.slice(mark).filter(batch => batch.checkpoints[0] === "edit" && batch.mode !== "log").map(batch => batch.edits[0]);
    const problems = left.flatMap((call, k) => callProblems(model, call, checks[k], told));
    return problems.length === 0 ? [] : [...problems, ...trace(model, checks, answers)];
  } finally {
    await service.close();
  }
}

/** What a failure report shows besides its problems: the files as they end, each late check's read, the answers. */
function trace(model: Model, checks: readonly (BatchEdit | undefined)[], answers: readonly string[]): string[] {
  const files = [...model.files].map(([path, lines]) => `file ${path}: ${JSON.stringify(lines.map(line => `${line.id}:${line.text.trim()}`))}`);
  const reads = checks.map((edit, k) => `late check ${k}: ${edit?.path} before ${JSON.stringify(edit?.previousText ?? edit?.previousReadError ?? "none")}`);
  return [...files, ...reads, `answers: ${JSON.stringify(answers)}`];
}

/** `sequence` with one of its steps left out, for each step. */
function smaller(sequence: Sequence): Sequence[] {
  const without = (steps: Step[], index: number) => steps.filter((_, k) => k !== index);
  return [
    ...sequence.hold.map((_, index) => ({ ...sequence, hold: without(sequence.hold, index) })),
    ...sequence.before.map((_, index) => ({ ...sequence, before: without(sequence.before, index) })),
  ];
}

/** The smallest sequence found by leaving steps out one at a time while the property still breaks, with its problems. */
async function shrunk(sequence: Sequence, problems: string[]): Promise<{ sequence: Sequence; problems: string[] }> {
  for (const candidate of smaller(sequence)) {
    if (candidate.hold.length === 0) continue;
    const found = await problemsOf(candidate);
    if (found.length > 0) return await shrunk(candidate, found);
  }
  return { sequence, problems };
}

async function failureOf(seed: number): Promise<string | undefined> {
  const sequence = sequenceOf(seed);
  const problems = await problemsOf(sequence);
  if (problems.length === 0) return undefined;
  const smallest = await shrunk(sequence, problems);
  return [`seed ${seed}:`, ...smallest.problems, JSON.stringify(smallest.sequence, undefined, 1)].join("\n");
}

describe("an engine service checking tool calls its hooks left while its daemon did not read", () => {
  const chunks = Array.from({ length: SEQUENCES / CHUNK }, (_, k) => FIRST_SEED + k * CHUNK);
  it.each(chunks)("tells every line a live check tells, and exactly those for a call read exact, over seeds from %i", async first => {
    for (let seed = first; seed < first + CHUNK; seed += 1) {
      const failure = await failureOf(seed);
      if (failure !== undefined) throw new Error(failure);
    }
  }, 600_000);
});
