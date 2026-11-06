/**
 * The alignment ledger: every unit the other refs' commits yielded, qualified by branch, as visible at a moment. A
 * commit's units count from the time it became public (`visibleAt`); a hand-off a later visible commit on the same
 * ref removes is closed. A commit two refs share (a branch cut from another) is counted once, under the ref where it
 * became visible first.
 */
import { identifiers, shortName } from "./symbols.js";
import type { ExtractedUnit, UnitAnchors } from "./units.js";

export interface LedgerCommit {
  readonly sha: string;
  readonly author: string;
  /** ISO time the commit became public. */
  readonly visibleAt: string;
  readonly units: readonly ExtractedUnit[];
  /** Hand-off keys the commit removes (`closedHandoffKeys`). */
  readonly closes: readonly string[];
}

export interface RefHistory {
  /** The full ref name (`refs/pull/12/head`, `refs/remotes/origin/feature`). */
  readonly ref: string;
  /** The short name units are qualified by. */
  readonly branch: string;
  readonly pr: number | null;
  /** Oldest first. */
  readonly commits: readonly LedgerCommit[];
}

export interface LedgerUnit extends ExtractedUnit {
  /** `<branch>@<sha prefix>#<kind>-<n>`: unique across the ledger. */
  readonly id: string;
  readonly ref: string;
  readonly branch: string;
  readonly pr: number | null;
  readonly author: string;
  readonly sha: string;
  readonly visibleAt: string;
}

/** What one commit touches: the files it changes and the identifiers its changed lines use. */
export interface Touch {
  readonly files: readonly string[];
  readonly symbols: ReadonlySet<string>;
}

/** The short branch name of a full ref: `refs/remotes/origin/a/b` is `a/b`, `refs/pull/12/head` is `pull/12`. */
export function branchName(ref: string): string {
  const pull = /^refs\/pull\/(\d+)\/head$/u.exec(ref);
  if (pull !== null) return `pull/${pull[1]}`;
  return ref.replace(/^refs\/(?:remotes\/[^/]+|heads|tags)\//u, "");
}

/** Units visible at `time`: each ref's commits public by then, with closed hand-offs left out. */
export function visibleUnits(histories: readonly RefHistory[], time: string): LedgerUnit[] {
  const at = Date.parse(time);
  const firstSeen = new Map<string, number>();
  for (const history of histories) {
    for (const commit of history.commits) {
      const visible = Date.parse(commit.visibleAt);
      if (visible <= at && visible < (firstSeen.get(commit.sha) ?? Number.POSITIVE_INFINITY)) firstSeen.set(commit.sha, visible);
    }
  }
  const claimed = new Set<string>();
  const units: LedgerUnit[] = [];
  for (const history of histories) {
    const open = new Map<string, LedgerUnit[]>();
    const own: LedgerUnit[] = [];
    for (const commit of history.commits) {
      const visible = Date.parse(commit.visibleAt);
      if (visible > at) continue;
      for (const key of commit.closes) {
        for (const unit of open.get(key) ?? []) {
          const position = own.indexOf(unit);
          if (position !== -1) own.splice(position, 1);
        }
        open.delete(key);
      }
      if (firstSeen.get(commit.sha) !== visible || claimed.has(commit.sha)) continue;
      claimed.add(commit.sha);
      commit.units.forEach((unit, index) => {
        const entry: LedgerUnit = {
          ...unit,
          id: `${history.branch}@${commit.sha.slice(0, 10)}#${unit.kind}-${index}`,
          ref: history.ref,
          branch: history.branch,
          pr: history.pr,
          author: commit.author,
          sha: commit.sha,
          visibleAt: commit.visibleAt,
        };
        own.push(entry);
        if (unit.key !== null) open.set(unit.key, [...(open.get(unit.key) ?? []), entry]);
      });
    }
    units.push(...own);
  }
  return units;
}

/** Whether `anchors` cover `path`: the same path, or a folder anchor (`src/a/`) above it. */
export function anchorsCoverFile(anchors: UnitAnchors, path: string): boolean {
  return anchors.files.some(file => (file.endsWith("/") ? path.startsWith(file) : file === path));
}

/** The anchors a touch hits: covered files, and symbols whose last segment the touch's changed lines use. */
export function touchedAnchors(anchors: UnitAnchors, touch: Touch): { files: string[]; symbols: string[] } {
  return {
    files: touch.files.filter(path => anchorsCoverFile(anchors, path)),
    symbols: anchors.symbols.filter(symbol => touch.symbols.has(shortName(symbol))),
  };
}

/**
 * Consumer lookup: of `units`, those whose symbols the given files use, by unit id, with the files and symbols that
 * matched. `texts` are the touched files' contents; a contract change reaches whoever calls what it changed.
 */
export function consumerHits(units: readonly LedgerUnit[], texts: ReadonlyMap<string, string>): Map<string, { files: string[]; symbols: string[] }> {
  const used = [...texts].map(([path, text]) => ({ path, names: identifiers(text) }));
  const hits = new Map<string, { files: string[]; symbols: string[] }>();
  for (const unit of units) {
    const symbols = unit.anchors.symbols.filter(symbol => used.some(file => file.names.has(shortName(symbol))));
    if (symbols.length === 0) continue;
    const files = used.filter(file => symbols.some(symbol => file.names.has(shortName(symbol)))).map(file => file.path);
    hits.set(unit.id, { files, symbols });
  }
  return hits;
}

/** A path's area as alignment groups work: its folder cut to two segments (`src/ui` for `src/ui/forms/a.ts`). */
export function pathArea(path: string): string {
  return path.split("/").slice(0, -1).slice(0, 2).join("/");
}

function words(text: string): Set<string> {
  return new Set(text.toLowerCase().split(/[^a-z0-9_]+/u).filter(token => token.length >= 4));
}

/**
 * Of `live`, the positions most worth comparing with `mine`, at most `max`: shared files and symbols count most, a
 * shared area less, then the words the texts share. Ones sharing nothing are left out.
 */
export function comparablePositions<T extends { text: string; anchors: UnitAnchors }>(mine: readonly { text: string; anchors: UnitAnchors }[], live: readonly T[], max: number): T[] {
  const mineWords = words(mine.map(unit => unit.text).join(" "));
  const mineFiles = new Set(mine.flatMap(unit => unit.anchors.files));
  const mineAreas = new Set([...mineFiles].map(pathArea));
  const mineSymbols = new Set(mine.flatMap(unit => unit.anchors.symbols.map(shortName)));
  const scored = live.map(unit => {
    let score = 0;
    for (const file of unit.anchors.files) score += mineFiles.has(file) ? 3 : mineAreas.has(pathArea(file)) ? 1 : 0;
    for (const symbol of unit.anchors.symbols) if (mineSymbols.has(shortName(symbol))) score += 3;
    const theirs = words(unit.text);
    let common = 0;
    for (const token of theirs) if (mineWords.has(token)) common += 1;
    score += common / Math.sqrt(Math.max(1, theirs.size));
    return { unit, score };
  });
  return scored.filter(entry => entry.score > 0.5).sort((a, b) => b.score - a.score).slice(0, max).map(entry => entry.unit);
}
