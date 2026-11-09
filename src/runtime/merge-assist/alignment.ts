/**
 * The judge-backed parts of in-flight alignment: reading design positions out of doc and commit-message passages,
 * ranking an area's candidate units for the work at hand, and comparing a branch's new positions with the positions
 * live on other branches. Every input reaches the judge fenced (`judge-prompts.ts`); every answer is validated, and a
 * position is kept only when its quote is in the passage and its anchors are names the passage writes.
 */
import { createHash, randomBytes } from "node:crypto";
import { comparablePositions, type LedgerUnit, pathArea } from "../../engine/merge-assist/alignment/ledger.js";
import { conceptIds, distinctiveSymbol, type ExtractedUnit, MAX_UNIT_TEXT, type NameIndex, type Passage, type PathResolver, subsystemAnchors } from "../../engine/merge-assist/alignment/units.js";
import { stableJsonText } from "../../engine/stable-json.js";
import type { AgentJudge } from "./agent-judge.js";
import {
  type CompareInput,
  compareJsonSchemaFor,
  compareVerdictSchema,
  MAX_COMPARE_ITEMS,
  positionComparePrompt,
  positionsJsonSchemaFor,
  positionsPrompt,
  positionsVerdictSchemaFor,
  type PositionsInput,
  type PositionsVerdict,
  rankJsonSchemaFor,
  rankPrompt,
  rankVerdictSchemaFor,
} from "./judge-prompts.js";

/** The most units delivered for one area at once. */
export const MAX_AREA_UNITS = 3;

function cacheKey(kind: string, inputs: unknown): string {
  return createHash("sha256").update(`alignment:${kind}\0${stableJsonText(inputs) ?? ""}`).digest("hex");
}

function flat(text: string): string {
  return text.replace(/\s+/gu, " ").trim().toLowerCase();
}

export interface PositionsOutcome {
  /** Position units per passage id; passages with none are absent. */
  readonly units: Map<string, ExtractedUnit[]>;
  /** Why the judge gave no answer, or null. */
  readonly error: string | null;
  /** Positions the judge answered that failed validation (a quote not in the passage). */
  readonly rejected: number;
}

/** The ref's tree as concept anchors read it: its folders and files by name, and the files that write an id. */
export interface ConceptTree {
  readonly names: NameIndex;
  /** Files in the tree that write `id` (`O41`, or a decision number `0035`), at most a dozen, most direct first. */
  referencing(id: string): readonly string[];
}

/** The most files one position is anchored to. */
const MAX_POSITION_FILES = 16;
/** A commit changing at most this many source files is focused: the code its positions are about. */
const MAX_FOCUSED_COMMIT_FILES = 6;

type ExtractInput = { repositoryKey: string; source: string; passages: readonly Passage[]; resolve: PathResolver; tree?: ConceptTree; changed?: readonly string[] };

/** Where a passage is from as a prompt and a unit name it: its doc, its source, or the commit message (`source`). */
function passageOrigin(passage: Passage, source: string): { prompt: string; unit: string } {
  const named = passage.path ?? passage.source;
  return named === undefined ? { prompt: `commit message (${source})`, unit: `commit message: ${source}` } : { prompt: named, unit: named };
}

/** One judged position of `passage` as a unit, anchored as {@link extractPositions} describes. */
function positionUnit(position: PositionsVerdict["positions"][number], passage: Passage, input: ExtractInput, focused: readonly string[]): ExtractedUnit {
  const files = position.files.filter(file => passage.text.includes(file)).flatMap(file => {
    const path = input.resolve(file);
    return path === undefined ? [] : [path];
  });
  const symbols = position.symbols
    .filter(symbol => passage.text.includes(symbol))
    .map(symbol => symbol.replace(/(?:\(\)|!)$/u, "").replace(/::/gu, "."))
    .filter(distinctiveSymbol);
  const ids = conceptIds(position.ids, `${passage.heading ?? ""}\n${passage.text}`);
  const area = passage.path === null ? "" : pathArea(passage.path);
  const tree = input.tree;
  const concepts = tree === undefined ? [] : [...subsystemAnchors(position.subsystem, tree.names), ...ids.flatMap(id => tree.referencing(id))];
  return {
    kind: "position",
    text: `${position.statement} ("${position.quote.replace(/\s+/gu, " ").trim()}", ${passageOrigin(passage, input.source).unit})`.slice(0, MAX_UNIT_TEXT),
    anchors: {
      files: [...new Set([...(passage.path === null ? [] : [passage.path]), ...files, ...(area === "" ? [] : [`${area}/`]), ...focused, ...concepts])].slice(0, MAX_POSITION_FILES),
      // Open-item ids are names the later side writes too; a decision is reached through its record's files.
      symbols: [...new Set([...symbols, ...ids.filter(id => id.startsWith("O"))])],
    },
    key: null,
  };
}

/**
 * The positions `passages` take, as units anchored to the files and symbols each passage names (resolved by
 * `resolve`), to the doc it is in and that doc's area (`docs/final-shape/`), to the source files the commit changes
 * (`changed`) when it is focused, and, given the ref's `tree`, to concept anchors: the folders and files its subsystem
 * names and the files that write the decision and open-item ids it belongs to. One judge call for all of them;
 * `source` labels the commit they come from, a passage's own `source` the description it is.
 */
export async function extractPositions(judge: AgentJudge, input: ExtractInput): Promise<PositionsOutcome> {
  if (input.passages.length === 0) return { units: new Map(), error: null, rejected: 0 };
  const focused = input.changed !== undefined && input.changed.length <= MAX_FOCUSED_COMMIT_FILES ? input.changed : [];
  const byId = new Map(input.passages.map((passage, index) => [`p${index + 1}`, passage]));
  const prompted: PositionsInput = {
    passages: [...byId].map(([id, passage]) => ({ id, source: passageOrigin(passage, input.source).prompt, heading: passage.heading, text: passage.text })),
  };
  const verdict = positionsVerdictSchemaFor(prompted);
  const outcome = await judge.judge({
    kind: "positions",
    repositoryKey: input.repositoryKey,
    prompt: positionsPrompt(prompted, randomBytes(16).toString("hex")),
    jsonSchema: positionsJsonSchemaFor(prompted),
    parse: value => verdict.parse(value),
    cacheKey: cacheKey("positions:concepts", prompted),
  });
  if (!outcome.ok) return { units: new Map(), error: `${outcome.reason}: ${outcome.detail}`, rejected: 0 };
  return { ...positionUnits(outcome.value.positions, byId, input, focused), error: null };
}

/** The judged positions whose quote is in their passage as units by passage id, and how many were not. */
function positionUnits(positions: PositionsVerdict["positions"], byId: ReadonlyMap<string, Passage>, input: ExtractInput, focused: readonly string[]): { units: Map<string, ExtractedUnit[]>; rejected: number } {
  const units = new Map<string, ExtractedUnit[]>();
  let rejected = 0;
  for (const position of positions) {
    const passage = byId.get(position.passageId)!;
    if (!flat(passage.text).includes(flat(position.quote))) {
      rejected += 1;
      continue;
    }
    units.set(passage.id, [...(units.get(passage.id) ?? []), positionUnit(position, passage, input, focused)]);
  }
  return { units, rejected };
}

export interface RankedUnits {
  readonly units: LedgerUnit[];
  /** `judge` ranked them; `fallback` kept the first candidates after the judge failed; `none` had no candidates. */
  readonly via: "judge" | "fallback" | "none";
  readonly error: string | null;
}

/** How a unit reads in a ranking or comparison prompt: its kind, its text and where it is from. */
export function unitLine(unit: LedgerUnit): string {
  return `[${unit.kind}] ${unit.text} (branch ${unit.branch}, ${unit.author})`;
}

/**
 * The units of `candidates` the work described by `task` most likely needs, most relevant first, at most `max`
 * (`rankPrompt`). Candidates are offered in the caller's order, which is also the fallback order when the judge fails.
 */
export async function rankUnits(
  judge: AgentJudge,
  input: { repositoryKey: string; task: string; candidates: readonly LedgerUnit[]; max?: number },
): Promise<RankedUnits> {
  const max = input.max ?? MAX_AREA_UNITS;
  if (input.candidates.length === 0) return { units: [], via: "none", error: null };
  const ids = input.candidates.map((_unit, index) => `u${index + 1}`);
  const decisions = input.candidates.map((unit, index) => ({ id: ids[index]!, decision: unitLine(unit) }));
  const verdict = rankVerdictSchemaFor(max);
  const outcome = await judge.judge({
    kind: "rank",
    repositoryKey: input.repositoryKey,
    prompt: rankPrompt({ task: input.task, decisions }, randomBytes(16).toString("hex"), max),
    jsonSchema: rankJsonSchemaFor(ids, max),
    parse: value => verdict.parse(value),
    cacheKey: cacheKey("rank", { task: input.task, decisions, max }),
  });
  if (!outcome.ok) return { units: input.candidates.slice(0, max), via: "fallback", error: `${outcome.reason}: ${outcome.detail}` };
  const picked = [...new Set(outcome.value.ids)].flatMap(id => {
    const index = ids.indexOf(id);
    return index === -1 ? [] : [input.candidates[index]!];
  });
  return { units: picked, via: "judge", error: null };
}

/** A position stated on the branch being worked on: the side a comparison calls "mine". */
export interface OwnPosition {
  readonly id: string;
  readonly text: string;
  readonly files: readonly string[];
}

export interface PositionVerdict {
  readonly theirs: LedgerUnit;
  readonly mine: OwnPosition | null;
  readonly relation: "compatible" | "duplicate" | "contradicts" | "unsure";
  readonly explanation: string;
  readonly message: string | null;
}

export interface ComparisonOutcome {
  readonly verdicts: PositionVerdict[];
  readonly error: string | null;
}

/**
 * How each of the positions live on other branches (`theirs`, at most `MAX_COMPARE_ITEMS`) relates to the positions
 * this branch just stated (`mine`), from one `positionComparePrompt` call.
 */
export async function comparePositions(
  judge: AgentJudge,
  input: { repositoryKey: string; mine: readonly OwnPosition[]; theirs: readonly LedgerUnit[] },
): Promise<ComparisonOutcome> {
  const theirs = input.theirs.slice(0, MAX_COMPARE_ITEMS);
  if (input.mine.length === 0 || theirs.length === 0) return { verdicts: [], error: null };
  const compare: CompareInput = {
    mine: input.mine.map((own, index) => ({ id: `candidate:m-${index + 1}`, decision: own.text, scope: [...own.files] })),
    theirs: theirs.map((unit, index) => ({ id: `candidate:t-${index + 1}`, decision: unit.text, scope: [...unit.anchors.files], author: unit.author })),
    conflicts: [],
  };
  const outcome = await judge.judge({
    kind: "compare",
    repositoryKey: input.repositoryKey,
    prompt: positionComparePrompt(compare, randomBytes(16).toString("hex")),
    jsonSchema: compareJsonSchemaFor(compare),
    parse: value => compareVerdictSchema.parse(value),
    // Keyed on the prompt's text (under a fixed nonce) too, so a change of instructions asks again.
    cacheKey: cacheKey("compare:positions", { compare, prompt: positionComparePrompt(compare, "0".repeat(32)) }),
  });
  if (!outcome.ok) return { verdicts: [], error: `${outcome.reason}: ${outcome.detail}` };
  const verdicts = outcome.value.items.flatMap(item => {
    const theirsIndex = compare.theirs.findIndex(peer => peer.id === item.candidateId);
    if (theirsIndex === -1) return [];
    const mineIndex = item.mineId === null ? -1 : compare.mine.findIndex(own => own.id === item.mineId);
    return [{
      theirs: theirs[theirsIndex]!,
      mine: mineIndex === -1 ? null : input.mine[mineIndex]!,
      relation: item.relation,
      explanation: item.explanation,
      message: item.message,
    }];
  });
  return { verdicts, error: null };
}

/** The most of one ref's positions, and of the other refs' comparable positions, one contest call carries. */
const MAX_CONTEST_MINE = 32;
const MAX_CONTEST_THEIRS = 48;

export interface PositionContest {
  /** Every pair the judge answered: `mine` is a position of the earlier-listed ref. */
  readonly pairs: Array<{ mine: LedgerUnit | null; theirs: LedgerUnit; relation: PositionVerdict["relation"]; explanation: string }>;
  /** Ids of positions in a contradicting pair. */
  readonly contested: Set<string>;
  readonly errors: string[];
}

/**
 * The compare step over positions live on different refs. Each ref in turn has its positions compared with the
 * comparable positions of the refs listed after it (`comparablePositions`: shared anchors, then shared words), one
 * `comparePrompt` call per ref, so each pair of refs is compared once. A position in a contradicting pair is contested.
 */
export async function contestPositions(judge: AgentJudge, input: { repositoryKey: string; positions: readonly LedgerUnit[] }): Promise<PositionContest> {
  const refs = [...new Set(input.positions.map(unit => unit.ref))];
  const pairs: PositionContest["pairs"] = [];
  const contested = new Set<string>();
  const errors: string[] = [];
  for (const [index, ref] of refs.entries()) {
    const mineUnits = input.positions.filter(unit => unit.ref === ref).slice(-MAX_CONTEST_MINE);
    const later = new Set(refs.slice(index + 1));
    const theirs = comparablePositions(mineUnits, input.positions.filter(unit => later.has(unit.ref)), MAX_CONTEST_THEIRS);
    if (theirs.length === 0) continue;
    const mine = mineUnits.map(unit => ({ id: unit.id, text: unit.text, files: unit.anchors.files }));
    const outcome = await comparePositions(judge, { repositoryKey: input.repositoryKey, mine, theirs });
    if (outcome.error !== null) errors.push(`${ref}: ${outcome.error}`);
    for (const verdict of outcome.verdicts) {
      // A compatible or unsure answer names no position of mine; with one, it is that one.
      const own = verdict.mine === null ? (mineUnits.length === 1 ? mineUnits[0]! : null) : (mineUnits.find(unit => unit.id === verdict.mine!.id) ?? null);
      pairs.push({ mine: own, theirs: verdict.theirs, relation: verdict.relation, explanation: verdict.explanation });
      if (verdict.relation === "contradicts" && own !== null) {
        contested.add(own.id);
        contested.add(verdict.theirs.id);
      }
    }
  }
  return { pairs, contested, errors };
}
