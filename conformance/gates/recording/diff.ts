/**
 * What changed between the recording a tool-pack had and the one the recorder just made, per upstream rule.
 *
 * Until the recordings left the tree, a re-record showed up in review as tens of thousands of changed JSON lines,
 * which nobody read and which said nothing about whether upstream's answers had actually moved. The recordings are
 * not in the diff at all now, so this is what takes their place: for each rule, how many expected findings upstream
 * gained or lost on its own tests, how many corpus files it now answers differently for, and which of its facts
 * changed. A rule that is absent here did not move, and a change in it is a reason to look at the rule.
 */

import type { GatesCorpusRecording, GatesRecording, RecordedSpan } from "./schema.js";

/** How one upstream rule's recorded answers moved. */
export interface RuleChange {
  rule: string;
  /** `added`: upstream has the rule and the last recording did not; `removed`: the other way round. */
  status: "added" | "removed" | "changed";
  /** Expected findings on upstream's own test files that the rule gained and lost. */
  expectedAdded: number;
  expectedRemoved: number;
  /** Corpus files whose findings for the rule differ, in either direction. */
  corpusFilesChanged: number;
  /** The fields of `rule().facts` that differ, sorted (`deprecation`, `recommended`, `tests`). */
  factsChanged: string[];
}

export interface RecordingChanges {
  pack: string;
  tool: string;
  /** Pins that moved, as `<field> <before> → <after>`; a re-record on unchanged pins has none. */
  pinsChanged: string[];
  /** True when the tool-pack had no recording at all: every rule is `added` and nothing is a change to review. */
  first: boolean;
  /**
   * Why the recording the committed lock named could not be read, when it could not. The tool-pack was recorded
   * before, so this is not a first recording: what moved is unknown, which is not the same as nothing moving.
   */
  unavailable: string | null;
  /** Every rule whose answers moved, sorted by name. */
  rules: RuleChange[];
  /** Totals, so a reader sees the shape of the recording even when no rule moved. */
  totals: { rules: number; cases: number; corpusFiles: number };
}

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** A case's findings as a multiset of positions, so two cases are compared by what upstream reported, not by order. */
function spanCounts(spans: RecordedSpan[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const span of spans) {
    const key = JSON.stringify(span);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return counts;
}

/** How many findings `after` has that `before` does not, and the other way round, over one rule's whole case list. */
function expectedDelta(before: RecordedSpan[], after: RecordedSpan[]): { added: number; removed: number } {
  const was = spanCounts(before);
  const now = spanCounts(after);
  let added = 0;
  let removed = 0;
  for (const key of new Set([...was.keys(), ...now.keys()])) {
    const difference = (now.get(key) ?? 0) - (was.get(key) ?? 0);
    if (difference > 0) added += difference;
    else removed -= difference;
  }
  return { added, removed };
}

/** A rule's expected findings across every one of its cases, keyed by case so a reordered case list is no change. */
function caseFindings(recording: GatesRecording, rule: string): Map<string, RecordedSpan[]> {
  const found = new Map<string, RecordedSpan[]>();
  for (const testCase of recording.rules[rule]?.cases ?? []) {
    found.set(`${testCase.fixture}\0${testCase.variant}\0${testCase.path}`, testCase.expected);
  }
  return found;
}

/** A rule's corpus findings by file, so a corpus file that was added or dropped counts as a change for the rule. */
function corpusFindings(corpus: GatesCorpusRecording | null, rule: string): Map<string, string> {
  const found = new Map<string, string>();
  for (const file of corpus?.files ?? []) {
    const spans = file.findings[rule];
    if (spans !== undefined) found.set(`${file.task}\0${file.path}`, JSON.stringify(spans));
  }
  return found;
}

export type RecordedPair = { recording: GatesRecording; corpus: GatesCorpusRecording | null };

/** A recording the committed lock names that could not be read, and why. */
export type UnreadRecording = { unavailable: string };

/**
 * The per-rule summary of a re-record. `before` is null when the tool-pack had no recording, and says why when the
 * lock named one that could not be read: nothing is compared then, and the summary says so.
 */
export function diffRecordings(pack: string, tool: string, before: RecordedPair | UnreadRecording | null, after: RecordedPair): RecordingChanges {
  const totals = {
    rules: Object.keys(after.recording.rules).length,
    cases: Object.values(after.recording.rules).reduce((count, rule) => count + rule.cases.length, 0),
    corpusFiles: after.corpus?.files.length ?? 0,
  };
  if (before === null) {
    return {
      pack,
      tool,
      pinsChanged: [],
      first: true,
      unavailable: null,
      rules: Object.keys(after.recording.rules).sort(byText).map((rule) => ({ rule, status: "added", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: [] })),
      totals,
    };
  }
  if ("unavailable" in before) return { pack, tool, pinsChanged: [], first: false, unavailable: before.unavailable, rules: [], totals };
  const pinsChanged = (["repository", "commit", "version", "runtime"] as const)
    .filter((field) => before.recording.pins[field] !== after.recording.pins[field])
    .map((field) => `${field} ${before.recording.pins[field]} → ${after.recording.pins[field]}`);

  const rules: RuleChange[] = [];
  for (const rule of [...new Set([...Object.keys(before.recording.rules), ...Object.keys(after.recording.rules)])].sort(byText)) {
    const was = before.recording.rules[rule];
    const now = after.recording.rules[rule];
    if (was === undefined || now === undefined) {
      rules.push({ rule, status: now === undefined ? "removed" : "added", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: [] });
      continue;
    }
    const wasCases = caseFindings(before.recording, rule);
    const nowCases = caseFindings(after.recording, rule);
    const places = new Set([...wasCases.keys(), ...nowCases.keys()]);
    const expected = [...places].reduce((total, place) => {
      const delta = expectedDelta(wasCases.get(place) ?? [], nowCases.get(place) ?? []);
      return { added: total.added + delta.added, removed: total.removed + delta.removed };
    }, { added: 0, removed: 0 });
    const wasCorpus = corpusFindings(before.corpus, rule);
    const nowCorpus = corpusFindings(after.corpus, rule);
    const corpusFilesChanged = [...new Set([...wasCorpus.keys(), ...nowCorpus.keys()])].filter((place) => wasCorpus.get(place) !== nowCorpus.get(place)).length;
    const factsChanged = (Object.keys(now.facts) as (keyof typeof now.facts)[])
      .filter((field) => JSON.stringify(was.facts[field]) !== JSON.stringify(now.facts[field]))
      .map(String)
      .sort(byText);
    if (expected.added + expected.removed + corpusFilesChanged + factsChanged.length === 0) continue;
    rules.push({ rule, status: "changed", expectedAdded: expected.added, expectedRemoved: expected.removed, corpusFilesChanged, factsChanged });
  }
  return { pack, tool, pinsChanged, first: false, unavailable: null, rules, totals };
}

/** Starts the recorder's comment, so CI finds the earlier ones to minimize, as the gate comment does. */
export const RECORDING_COMMENT_MARKER = "<!-- rerail-gate-recording -->";

/** At most this many rule rows; the rest are counted. A tool-pack has up to a few hundred rules. */
const ROW_LIMIT = 60;

/**
 * The summary as Markdown, for the pull request the recording was made for. It leads with whether upstream's answers
 * moved at all, because that is the one thing a reviewer needs from a re-record: a new recording whose answers are
 * the same is a recording of the same upstream, and a rule whose expected findings moved is a rule to look at.
 */
export function renderRecordingChanges(changes: RecordingChanges[]): string {
  const moved = changes.filter((entry) => entry.rules.length > 0 || entry.pinsChanged.length > 0);
  // A tool-pack that was not compared is never counted as answering the same: its changes are unknown.
  const uncompared = changes.filter((entry) => entry.unavailable !== null);
  const packs = (count: number) => `${count} tool-pack${count === 1 ? "" : "s"}`;
  const headline = [...(uncompared.length > 0 ? [`⚠️ ${packs(uncompared.length)} not compared`] : []), ...(moved.length > 0 ? [`${packs(moved.length)} moved`] : [])];
  const lines = [
    RECORDING_COMMENT_MARKER,
    `## Gate recordings: ${headline.length === 0 ? "✅ re-recorded, and upstream answers the same" : headline.join(", ")}`,
    "",
    "Upstream's recorded answers are not in the diff (`recording.lock` names them; docs/rule-gates.md). This is what changed in them.",
    "",
  ];
  for (const entry of changes) {
    const label = `\`${entry.pack}/${entry.tool}\``;
    const shape = `${entry.totals.rules} rules, ${entry.totals.cases} upstream cases, ${entry.totals.corpusFiles} corpus files`;
    if (entry.unavailable !== null) {
      lines.push(
        `### ${label}: ⚠️ not compared`,
        "",
        `${shape}. The recording the committed \`recording.lock\` named could not be read (${entry.unavailable}), so what moved is unknown. This is not a first recording, and not a recording that answers the same.`,
        "",
      );
      continue;
    }
    if (entry.first) {
      lines.push(`### ${label}: first recording`, "", `${shape}. Nothing to compare it with.`, "");
      continue;
    }
    if (entry.rules.length === 0 && entry.pinsChanged.length === 0) {
      lines.push(`### ${label}: unchanged`, "", `${shape}. Upstream answers exactly as it did.`, "");
      continue;
    }
    lines.push(`### ${label}`, "", `${shape}.`, "");
    if (entry.pinsChanged.length > 0) lines.push(`Pins moved: ${entry.pinsChanged.map((pin) => `\`${pin}\``).join(", ")}.`, "");
    lines.push(
      "| Upstream rule | | Expected findings | Corpus files | Facts |",
      "| --- | --- | --- | --- | --- |",
      ...entry.rules.slice(0, ROW_LIMIT).map((rule) => [
        `\`${rule.rule}\``,
        rule.status,
        rule.expectedAdded + rule.expectedRemoved === 0 ? "—" : `+${rule.expectedAdded} / −${rule.expectedRemoved}`,
        rule.corpusFilesChanged === 0 ? "—" : String(rule.corpusFilesChanged),
        rule.factsChanged.length === 0 ? "—" : rule.factsChanged.map((field) => `\`${field}\``).join(", "),
      ].join(" | ")).map((row) => `| ${row} |`),
      ...(entry.rules.length > ROW_LIMIT ? [`| … | ${entry.rules.length - ROW_LIMIT} more | | | |`] : []),
      "",
    );
  }
  return `${lines.join("\n").trimEnd()}\n`;
}
