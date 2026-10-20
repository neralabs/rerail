#!/usr/bin/env node
// The tool-packs port-readiness-record.yml records (docs/port-readiness.md, "Recording"). Under the
// `record-port-readiness` label, only the ones whose recorder inputs the pull request changes: what the recorder
// (conformance/recorders/port-readiness/record.ts) reads from the head's packs/ and corpus-tasks.json, diffed from the
// merge base of the base commit and the head to the head, and compared as the recorder reads it. A dispatch keeps its
// pack and tool filter. authorize runs this from the base commit's checkout on plain Node, before anything is
// installed, with the head fetched as git objects and never checked out: it imports nothing but node: builtins, and
// runs nothing the pull request wrote.
//
//   node conformance/port-readiness/recording/record-plan.mjs --event pull_request_target \
//     --base <sha> --head <sha> --tool-packs <json> [--comment <file>]
//   node conformance/port-readiness/recording/record-plan.mjs --event workflow_dispatch \
//     --pack <pack> --tool <tool> --tool-packs <json> [--comment <file>]
//
// --tool-packs is main's TOOL_PACKS, the only tool-packs a run records; --comment writes a Markdown body for the pull
// request saying what is recorded and why, or why nothing is. Prints a ::warning for each change a label cannot
// record and one line per tool-pack and why. When $GITHUB_OUTPUT is set, appends `legs`, the record job's matrix
// include list, as compact JSON.
import { execFileSync } from "node:child_process";
import { appendFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

/** An UPSTREAM.md line naming an upstream path prefix nothing is copied from: record.ts's NOT_COPIED, as a test holds. */
export const NOT_COPIED = /^- Not copied: `([^`]+)`/gmu;

const CORPUS_TASKS = "conformance/port-readiness/corpus-tasks.json";
const RECORDER = "conformance/recorders/port-readiness/";
const RUNTIMES = `${RECORDER}runtimes/`;
/** A tool-pack's own recorder inputs: its manifest, its committed case sources, its goldens. */
const TOOL_PACK_INPUT = /^packs\/([^/]+)\/upstream\/([^/]+)\/(gates\.yaml|goldens\.tsv|fixtures\/.+)$/su;
/** The conformance manifests the recorder reads its rule names and corpora from (corpus.ts conformancePaths). */
const MANIFEST = /^packs\/([^/]+)\/(?:candidates\/)?conformance\.json$/u;
/** The files the recorder reads the "Not copied" prefixes from (record.ts notCopiedPrefixes). */
const UPSTREAM_MD = /^packs\/([^/]+)\/licenses\/[^/]+\/UPSTREAM\.md$/u;
const NAME = /^[a-z0-9][a-z0-9._-]*$/u;

const nameOf = ({ pack, tool }) => `${pack}/${tool}`;
const gatesOf = ({ pack, tool }) => `packs/${pack}/upstream/${tool}/gates.yaml`;
const conformancePaths = (pack) => [`packs/${pack}/conformance.json`, `packs/${pack}/candidates/conformance.json`];
const sortedUnique = (values) => [...new Set(values.filter((value) => value !== ""))].sort();
const firstOf = (items) => `${items[0]}${items.length === 1 ? "" : ` (and ${items.length - 1} more)`}`;
const listed = (items) => `${items.slice(0, 5).join(", ")}${items.length > 5 ? ` and ${items.length - 5} more` : ""}`;
const plural = (count) => `${count} tool-pack${count === 1 ? "" : "s"}`;
/**
 * Text the pull request chose (a path, a parse error quoting its file), control characters escaped: a file named with
 * a newline must not start a workflow command on stdout or a Markdown block in the comment.
 */
const shown = (text) => text.replace(/[\u0000-\u001f\u007f]/gu, (character) => `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`);

/** main's TOOL_PACKS: a JSON list of `{"pack", "tool"}`. Anything else throws: it is the whole of what may record. */
export function parseToolPacks(text) {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new Error(`TOOL_PACKS is not JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
  if (!Array.isArray(parsed) || !parsed.every((entry) => NAME.test(entry?.pack ?? "") && NAME.test(entry?.tool ?? ""))) {
    throw new Error('TOOL_PACKS is not a list of {"pack": <pack>, "tool": <tool>} entries');
  }
  return parsed.map(({ pack, tool }) => ({ pack, tool }));
}

/** The prefixes an UPSTREAM.md lists as "Not copied", sorted, as record.ts notCopiedPrefixes reads them. */
export function notCopiedOf(text) {
  return [...text.matchAll(NOT_COPIED)].map((match) => match[1]).sort();
}

/** git in `root`, reading every path it is given as that path and no pattern. */
function git(root, args) {
  return execFileSync("git", ["--literal-pathspecs", ...args], {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 256 * 1024 * 1024,
  });
}

/** A file's text at a commit, or null when the commit has no such file; throws when git cannot read the commit. */
function show(root, commit, path) {
  const [entry = ""] = git(root, ["ls-tree", "-z", commit, "--", path]).split("\0");
  const [meta = "", name] = entry.split("\t");
  const [, type, object] = meta.split(" ");
  return type === "blob" && name === path ? git(root, ["cat-file", "blob", object]) : null;
}

/** Which of these paths the commit has. */
function present(root, commit, paths) {
  return new Set(paths.length === 0 ? [] : git(root, ["ls-tree", "-z", "--name-only", commit, "--", ...paths]).split("\0"));
}

/** A conformance manifest that is not JSON: the recorder could not read it either. */
class Unparsed extends Error {
  constructor(path, commit, error) {
    super(error instanceof Error ? error.message : String(error));
    this.path = path;
    this.commit = commit;
  }
}

/**
 * What the recorder reads of a pack's two conformance manifests at a commit: each tool's upstream rule names, merged
 * over both and sorted (record.ts conformanceRuleNames), and the corpora, paths and exclude sorted and merged by value
 * (corpus.ts readRerailCorpora). A rule or corpus moving from one manifest to the other changes neither.
 */
function readManifests(root, commit, pack) {
  const rules = new Map();
  const corpora = new Set();
  const sorted = (value) => (Array.isArray(value) ? [...value].sort() : value);
  for (const path of conformancePaths(pack)) {
    const text = show(root, commit, path);
    if (text === null) continue;
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch (error) {
      throw new Unparsed(path, commit, error);
    }
    for (const entry of Array.isArray(parsed?.rules) ? parsed.rules : []) {
      const { tool, rule } = entry?.upstream ?? {};
      if (typeof tool === "string" && typeof rule === "string") rules.set(tool, [...(rules.get(tool) ?? []), rule]);
    }
    for (const entry of Array.isArray(parsed?.corpora) ? parsed.corpora : []) {
      const { name, repository, commit: pinned, paths, exclude } = entry ?? {};
      corpora.add(JSON.stringify({ name, repository, commit: pinned, paths: sorted(paths), exclude: sorted(exclude ?? []) }));
    }
  }
  return { names: (tool) => sortedUnique(rules.get(tool) ?? []), corpora: [...corpora].sort() };
}

/** The "Not copied" prefixes of every packs/<pack>/licenses/<project>/UPSTREAM.md at a commit, as one sorted set. */
function notCopied(root, commit, pack) {
  const files = git(root, ["ls-tree", "-r", "-z", "--name-only", commit, "--", `packs/${pack}/licenses/`]).split("\0");
  const upstreams = files.filter((path) => UPSTREAM_MD.exec(path)?.[1] === pack);
  return sortedUnique(upstreams.flatMap((path) => notCopiedOf(show(root, commit, path) ?? "")));
}

/** The plan from each tool-pack's reasons: one with any is recorded, unless `dropped` says why not. */
function planOf(event, head, toolPacks, reasons, dropped, warnings, every = false) {
  const selected = toolPacks.filter((toolPack) => reasons.get(nameOf(toolPack)).length > 0);
  const chosen = selected.filter((toolPack) => !dropped.has(nameOf(toolPack)))
    .map((toolPack) => ({ ...toolPack, why: firstOf(reasons.get(nameOf(toolPack))) }));
  const gone = selected.filter((toolPack) => dropped.has(nameOf(toolPack))).map((toolPack) => ({ ...toolPack, why: dropped.get(nameOf(toolPack)) }));
  const lines = [
    ...chosen.map((entry) => `${nameOf(entry)}: ${entry.why}`),
    ...gone.map((entry) => `${nameOf(entry)}: not recorded, ${entry.why}`),
  ];
  if (event === "label" && chosen.length === 0) {
    lines.push(gone.length === 0
      ? "No tool-pack's recorder inputs changed: nothing to record."
      : "Every tool-pack the change selects is gone at the head: nothing to record.");
  }
  return { event, head, every, legs: chosen.map(({ pack, tool }) => ({ pack, tool })), chosen, dropped: gone, warnings, lines };
}

/** A dispatch's tool-packs: the ones its pack and tool name, an empty one naming every one, as the jq filter did. */
export function dispatchPlan(toolPacks, pack, tool) {
  const reason = `the dispatch names ${pack === "" ? "every pack" : `pack ${pack}`} and ${tool === "" ? "every tool" : `tool ${tool}`}`;
  const named = (entry) => (pack === "" || entry.pack === pack) && (tool === "" || entry.tool === tool);
  return planOf("dispatch", "", toolPacks, new Map(toolPacks.map((entry) => [nameOf(entry), named(entry) ? [reason] : []])), new Map(), []);
}

/**
 * A label's tool-packs: those of `toolPacks` whose recorder inputs the change from the merge base of `base` and `head`
 * to `head` touches, and whose gates.yaml `head` still has. A tool-pack's gates.yaml, fixtures/ or goldens.tsv
 * selects it; its tool's upstream rule names in the pack's conformance manifests select it, and the pack's corpora
 * there, a manifest that does not parse (with a warning) or the pack's "Not copied" prefixes select every tool-pack of
 * the pack; corpus-tasks.json selects every one. Whatever git cannot read selects every one, with a warning: leaving
 * out a tool-pack the change needs recorded is the failure to avoid. A change no label can record warns: a new
 * tool-pack's gates.yaml, and the recorder or its runtimes, which a label takes from main.
 */
export function labelPlan(toolPacks, base, head, root = ROOT) {
  const reasons = new Map(toolPacks.map((entry) => [nameOf(entry), []]));
  const add = (entries, reason) => {
    for (const entry of entries) reasons.get(nameOf(entry)).push(reason);
  };
  const ofPack = (pack) => toolPacks.filter((entry) => entry.pack === pack);
  const warnings = [];
  const dropped = new Map();
  try {
    const mergeBase = git(root, ["merge-base", base, head]).trim();
    const paths = git(root, ["diff", "--name-only", "--no-renames", "-z", mergeBase, head]).split("\0").filter((path) => path !== "");
    const manifests = new Set();
    const licenses = new Set();
    const unlisted = [];
    const runtimes = [];
    const recorder = [];
    for (const path of paths) {
      if (path === CORPUS_TASKS) add(toolPacks, `${path} changed`);
      const input = TOOL_PACK_INPUT.exec(path);
      const toolPack = input === null ? undefined : toolPacks.find((entry) => entry.pack === input[1] && entry.tool === input[2]);
      if (toolPack !== undefined) add([toolPack], `${shown(path)} changed`);
      else if (input?.[3] === "gates.yaml") unlisted.push(path);
      const manifest = MANIFEST.exec(path)?.[1];
      if (manifest !== undefined && ofPack(manifest).length > 0) manifests.add(manifest);
      const license = UPSTREAM_MD.exec(path)?.[1];
      if (license !== undefined && ofPack(license).length > 0) licenses.add(license);
      if (path.startsWith(RUNTIMES)) runtimes.push(path);
      else if (path.startsWith(RECORDER)) recorder.push(path);
    }
    for (const pack of manifests) {
      let before;
      let after;
      try {
        before = readManifests(root, mergeBase, pack);
        after = readManifests(root, head, pack);
      } catch (error) {
        if (!(error instanceof Unparsed)) throw error;
        const at = `${error.path} does not parse at ${error.commit.slice(0, 10)}`;
        warnings.push(`${at}, so every tool-pack of ${pack} is recorded: ${shown(error.message)}`);
        add(ofPack(pack), at);
        continue;
      }
      const where = `packs/${pack}/conformance.json and candidates/conformance.json`;
      for (const entry of ofPack(pack)) {
        const was = before.names(entry.tool);
        const is = after.names(entry.tool);
        const added = is.filter((name) => !was.includes(name));
        const removed = was.filter((name) => !is.includes(name));
        const delta = [...(added.length === 0 ? [] : [`added ${listed(added)}`]), ...(removed.length === 0 ? [] : [`removed ${listed(removed)}`])];
        if (delta.length > 0) add([entry], `the ${entry.tool} rules ${where} name changed (${delta.join("; ")})`);
      }
      if (before.corpora.join("\n") !== after.corpora.join("\n")) add(ofPack(pack), `the corpora ${where} pin changed`);
    }
    for (const pack of licenses) {
      if (notCopied(root, mergeBase, pack).join("\n") !== notCopied(root, head, pack).join("\n")) {
        add(ofPack(pack), `the "Not copied" lines of packs/${pack}/licenses/*/UPSTREAM.md changed`);
      }
    }
    // A tool-pack counts only while it is one at the head: its record leg could not read a gates.yaml that is gone.
    const selected = toolPacks.filter((entry) => reasons.get(nameOf(entry)).length > 0);
    const there = present(root, head, [...selected.map(gatesOf), ...unlisted]);
    for (const entry of selected) if (!there.has(gatesOf(entry))) dropped.set(nameOf(entry), `${gatesOf(entry)} is gone at the head`);
    for (const path of unlisted.filter((path) => there.has(path))) {
      warnings.push(`${shown(path)} is not a tool-pack main's port-readiness-record.yml records (TOOL_PACKS): a label records only `
        + "main's tool-packs, so a new tool-pack lands on main first, with its TOOL_PACKS entry, and main is then recorded");
    }
    if (runtimes.length > 0) {
      warnings.push(`${firstOf(runtimes.map(shown))} changed: a label records with main's runtimes, so a runtime change lands on main `
        + "first, and main is then recorded");
    }
    if (recorder.length > 0) {
      warnings.push(`${firstOf(recorder.map(shown))} changed: a label records with main's recorder, so this change records nothing until `
        + "it is on main");
    }
  } catch (error) {
    const stderr = typeof error?.stderr === "string" ? error.stderr.trim() : "";
    const reason = (stderr !== "" ? stderr : error instanceof Error ? error.message : String(error)).split("\n")[0];
    const every = new Map(toolPacks.map((entry) => [nameOf(entry), ["every tool-pack is recorded, since git could not compare the commits"]]));
    const warning = `git could not compare ${shown(base)} with ${shown(head)}, so every tool-pack is recorded: ${shown(reason)}`;
    return planOf("label", head, toolPacks, every, new Map(), [warning], true);
  }
  return planOf("label", head, toolPacks, reasons, dropped, warnings);
}

/** The pull request comment: what a run records and why, or why a label records nothing, and the warnings. */
export function commentBody(planned) {
  const at = `\`${planned.head.slice(0, 10)}\``;
  const items = (entries) => entries.map((entry) => `- \`${nameOf(entry)}\`: ${entry.why}`);
  const count = plural(planned.chosen.length);
  const lines = [];
  if (planned.event === "dispatch") {
    lines.push(`**Port readiness recording**: this run records the ${count} the dispatch names:`, "", ...items(planned.chosen));
  } else if (planned.every) {
    lines.push(`**Port readiness recording**: this run records every tool-pack, the ${count} main's \`port-readiness-record.yml\` `
      + `lists, at ${at}: git could not compare this pull request's head with \`main\`.`);
  } else if (planned.chosen.length > 0) {
    const intro = `**Port readiness recording**: this run records the ${count} whose recorder inputs this pull request changes, at ${at}:`;
    lines.push(intro, "", ...items(planned.chosen));
  } else {
    lines.push(`**Port readiness recording**: nothing to record at ${at}. A \`record-port-readiness\` label records only the `
      + "tool-packs main's `port-readiness-record.yml` lists whose recorder inputs the pull request changes: a tool-pack's "
      + "`gates.yaml`, a file under its `fixtures/`, or its `goldens.tsv`; its tool's upstream rules, or the pack's `corpora`, "
      + "in the pack's `conformance.json` or `candidates/conformance.json`; the `- Not copied:` lines of the pack's "
      + "`licenses/*/UPSTREAM.md`; or `conformance/port-readiness/corpus-tasks.json`. "
      + (planned.dropped.length === 0
        ? "This pull request changes none of them, so no tool-pack is recorded."
        : "The tool-packs it changes them for are gone at the head, so none is recorded."));
  }
  if (planned.dropped.length > 0) lines.push("", "Not recorded:", "", ...items(planned.dropped));
  if (planned.warnings.length > 0) lines.push("", "Warnings:", "", ...planned.warnings.map((warning) => `- ${warning}`));
  if (planned.event === "label" && planned.chosen.length === 0) {
    lines.push("", "To record a tool-pack anyway, run `port-readiness-record.yml` from `main` by hand with this pull request's number, "
      + "its head `sha`, and the `pack` and `tool`.");
  }
  return `${lines.join("\n")}\n`;
}

/** Appends the plan to a GitHub Actions output file: exactly `legs=<json>`. */
export function writeOutputs(file, planned) {
  appendFileSync(file, `legs=${JSON.stringify(planned.legs)}\n`);
}

const USAGE = "usage: node conformance/port-readiness/recording/record-plan.mjs --event <event> --tool-packs <json> "
  + "[--base <sha> --head <sha>] [--pack <pack>] [--tool <tool>] [--comment <file>]\n";

function main(args) {
  const options = Object.fromEntries(["event", "base", "head", "pack", "tool", "tool-packs", "comment"].map((name) => [name, { type: "string" }]));
  let values;
  try {
    ({ values } = parseArgs({ args, options, strict: true }));
  } catch {
    values = {};
  }
  if (values.event === undefined || values["tool-packs"] === undefined) {
    process.stderr.write(USAGE);
    return 2;
  }
  let toolPacks;
  try {
    toolPacks = parseToolPacks(values["tool-packs"]);
  } catch (error) {
    console.log(`::error::${error.message}`);
    return 2;
  }
  const pack = values.pack ?? "";
  const tool = values.tool ?? "";
  const planned = values.event === "pull_request_target"
    ? labelPlan(toolPacks, values.base ?? "", values.head ?? "")
    : dispatchPlan(toolPacks, pack, tool);
  if (planned.event === "dispatch" && planned.legs.length === 0) {
    console.log(`::error::no gated tool-pack is pack '${pack}', tool '${tool}'`);
    return 1;
  }
  // A workflow command's message escapes %, CR and LF (GitHub's escapeData).
  const escaped = (message) => message.replace(/%/gu, "%25").replace(/\r/gu, "%0D").replace(/\n/gu, "%0A");
  for (const warning of planned.warnings) console.log(`::warning title=Port readiness recording::${escaped(warning)}`);
  for (const line of planned.lines) console.log(line);
  if (values.comment !== undefined && values.comment !== "") writeFileSync(values.comment, commentBody(planned));
  if (process.env.GITHUB_OUTPUT !== undefined && process.env.GITHUB_OUTPUT !== "") writeOutputs(process.env.GITHUB_OUTPUT, planned);
  return 0;
}

// Imported by its tests, run as a script by CI: only the second plans.
const invoked = (() => {
  try {
    return process.argv[1] !== undefined && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (invoked) process.exitCode = main(process.argv.slice(2));
