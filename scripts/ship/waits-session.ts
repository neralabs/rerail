// The coding-agent sessions the user waits replay (waits.ts), and the pinned inputs they need (waits.json). No private
// session is replayed: a session is synthesized from a public repository's history, each first-parent commit one turn
// and each source file it adds or changes one Write of the file's new content, in commit order. It starts from a
// pinned real-code corpus (corpora.json) at the history's first commit, so the edits apply to the files they change.
// A session is built where the history is (a git checkout: `waits --prepare`), kept outside the repository, and pinned
// by the sha256 of its JSON in waits.json; a run refuses one that does not match.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, extname } from "node:path";

export interface SessionSpec {
  kind: "git-history";
  /** The public repository the history is read from, recorded for the reader; the build reads a local clone. */
  source: string;
  /** The waits.json repo the session edits: its corpus is the history at `from`. */
  repo: string;
  from: string;
  to: string;
  paths: string[];
  extensions: string[];
  maxEdits: number;
  /** The agent's pauses, replayed between events: after an edit, and around a turn's prompt and stop. */
  pauses: { edit_ms: number; turn_ms: number };
  edits: number | null;
  sha256: string | null;
}

export interface RepoSpec {
  /** A real-code corpus of corpora.json, checked against its pinned hash, copied into a fresh git repository. */
  corpus: string;
  /** The packs set up in the repository: what `rerail start` recommends there (its default configuration). */
  packs: string[];
  /** The rule `rerail rules show` reads (W7). */
  rule: string;
}

export interface WaitsManifest {
  format: 1;
  repos: Record<string, RepoSpec>;
  sessions: Record<string, SessionSpec>;
  /** Absolute limits (PRD §6.3); null until set. */
  limits: Record<string, number | null>;
  /** Waits reported but not counted (PRD §12.6 leaves W4 open). */
  reportOnly: string[];
}

export interface SessionEdit {
  path: string;
  /** The file's content after the edit. */
  content: string;
}

export interface Session {
  format: 1;
  name: string;
  source: string;
  from: string;
  to: string;
  pauses: SessionSpec["pauses"];
  turns: { commit: string; edits: SessionEdit[] }[];
}

function git(dir: string, args: readonly string[]): string {
  const result = spawnSync("git", ["-C", dir, ...args], { encoding: "utf8", maxBuffer: 1 << 30 });
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} in ${dir}: ${result.stderr.trim()}`);
  return result.stdout;
}

/** The session `spec` describes, from the history in the git checkout `gitDir`. */
export function buildSession(name: string, spec: SessionSpec, gitDir: string): Session {
  const commits = git(gitDir, ["rev-list", "--first-parent", "--reverse", `${spec.from}..${spec.to}`]).split("\n").filter((line) => line !== "");
  const turns: Session["turns"] = [];
  let edits = 0;
  for (const commit of commits) {
    if (edits >= spec.maxEdits) break;
    const changed = git(gitDir, ["diff-tree", "--no-commit-id", "-r", "--name-status", "--no-renames", `${commit}^1`, commit, "--", ...spec.paths])
      .split("\n")
      .map((line) => line.split("\t"))
      .filter(([status, path]) => (status === "A" || status === "M") && path !== undefined && spec.extensions.includes(extname(path)))
      .map(([, path]) => path!)
      .sort();
    const turn: SessionEdit[] = [];
    for (const path of changed.slice(0, spec.maxEdits - edits)) turn.push({ path, content: git(gitDir, ["show", `${commit}:${path}`]) });
    edits += turn.length;
    if (turn.length > 0) turns.push({ commit, edits: turn });
  }
  return { format: 1, name, source: spec.source, from: spec.from, to: spec.to, pauses: spec.pauses, turns };
}

export function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

export function editCount(session: Session): number {
  return session.turns.reduce((sum, turn) => sum + turn.edits.length, 0);
}

/** Writes the session to `path`; with `record`, pins its hash and edit count in the manifest, else checks them. */
export function prepareSession(manifestPath: string, name: string, gitDir: string, path: string, record: boolean): Session {
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as WaitsManifest;
  const spec = manifest.sessions[name];
  if (spec === undefined) throw new Error(`waits.json declares no session ${name}`);
  const session = buildSession(name, spec, gitDir);
  const text = `${JSON.stringify(session)}\n`;
  const hash = sha256(text);
  if (record) {
    spec.sha256 = hash;
    spec.edits = editCount(session);
    writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  } else if (spec.sha256 !== hash) {
    throw new Error(`session ${name} built to ${hash}; waits.json pins ${spec.sha256 ?? "nothing"} (pin it with --record)`);
  }
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
  return session;
}

/** The session at `path`, refused unless it is the one waits.json pins. */
export function loadSession(spec: SessionSpec, name: string, path: string): Session {
  if (!existsSync(path)) throw new Error(`${path} is missing: build it with \`shipbar.ts waits --prepare\` where the history is (a git checkout), then copy it here`);
  const text = readFileSync(path, "utf8");
  const hash = sha256(text);
  if (spec.sha256 === null || hash !== spec.sha256) throw new Error(`${path} is ${hash}; waits.json pins session ${name} at ${spec.sha256 ?? "nothing"}`);
  return JSON.parse(text) as Session;
}
