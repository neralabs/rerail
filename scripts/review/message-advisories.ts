/**
 * Check 8's message advisories (docs/pack-format.md, "Message") for the rule documents a pull request adds or changes:
 * the markdown CI's Message advisories job posts on the pull request and writes to its job summary. Advisories fail
 * nothing; the pack validation (test/engine/catalog/packs.test.ts) lists them for every document too.
 *
 *   node --import tsx scripts/review/message-advisories.ts [--base <commit> | --all] [--out <file>]
 *
 * `--base` takes the documents changed between that commit and HEAD; the default, HEAD^1, is the base side of the merge
 * commit a pull request's CI checks out. `--all` takes every rule document. The markdown is empty when no document has
 * an advisory.
 */
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { messageAdvisories } from "../../conformance/pack-validation.js";
import { CONFORMANCE_FILE, loadPack, RERAIL_ROOT } from "../../conformance/languages/pack.js";

/** The first line of the pull request comment, by which CI finds and collapses the earlier ones. */
export const COMMENT_MARKER = "<!-- rerail-message-advisories -->";

/** A rule document, shipped or candidate: `packs/<id>/rules/*.json` or `packs/<id>/candidates/rules/*.json`, not a probe. */
const RULE_DOCUMENT = /^packs\/([^/]+)\/((?:candidates\/)?rules\/[^/]+\.json)$/u;

export interface DocumentAdvisories {
  /** Repository-relative. */
  file: string;
  /** Each advisory, without the file it is about. */
  advisories: string[];
}

/** The rule documents among `files`, repository-relative paths, in order. */
export function ruleDocuments(files: readonly string[]): string[] {
  return files.filter(file => RULE_DOCUMENT.test(file));
}

/** Every rule document under `root`'s packs/. */
function allRuleDocuments(root: string): string[] {
  const packs = join(root, "packs");
  return readdirSync(packs, { withFileTypes: true })
    .filter(entry => entry.isDirectory())
    .flatMap(entry => ["rules", "candidates/rules"].map(folder => `packs/${entry.name}/${folder}`))
    .filter(folder => existsSync(join(root, folder)))
    .flatMap(folder => readdirSync(join(root, folder)).map(name => `${folder}/${name}`))
    .filter(file => RULE_DOCUMENT.test(file))
    .sort();
}

/** The rule documents changed between `base` and HEAD, deletions left out. */
function changedRuleDocuments(root: string, base: string): string[] {
  const output = execFileSync("git", ["diff", "--name-only", "--diff-filter=d", base, "HEAD"], { cwd: root, encoding: "utf8" });
  return ruleDocuments(output.split("\n").filter(line => line !== "")).sort();
}

/** The upstream rule each document of a pack ports, by its pack-relative path; none for a pack without conformance.json. */
function upstreamRules(packDirectory: string): Map<string, string> {
  if (!existsSync(join(packDirectory, CONFORMANCE_FILE))) return new Map();
  return new Map(loadPack(packDirectory).pack.rules.map(port => [port.policy.replace(/^\.\//u, ""), port.upstream.rule]));
}

/** Each of `files` (repository-relative rule documents under `root`) that has an advisory, with its advisories. */
export function documentAdvisories(root: string, files: readonly string[]): DocumentAdvisories[] {
  const ports = new Map<string, Map<string, string>>();
  return files.flatMap(file => {
    const [, pack = "", document = ""] = RULE_DOCUMENT.exec(file) ?? [];
    if (!ports.has(pack)) ports.set(pack, upstreamRules(join(root, "packs", pack)));
    const advisories = messageAdvisories(file, readFileSync(join(root, file), "utf8"), ports.get(pack)?.get(document));
    return advisories.length === 0 ? [] : [{ file, advisories: advisories.map(advisory => advisory.slice(file.length + 2)) }];
  });
}

/** Text a comment shows as written: angle brackets would read as HTML (`<reason>`). */
function escaped(text: string): string {
  return text.replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

/**
 * The pull request comment and job summary, or "" when there is nothing to advise on. `all`: the documents are every
 * rule document, not a pull request's.
 */
export function renderAdvisories(documents: readonly DocumentAdvisories[], all = false): string {
  if (documents.length === 0) return "";
  const count = documents.reduce((total, document) => total + document.advisories.length, 0);
  const subject = all ? "every rule pack's rule messages" : "the rule messages this pull request adds or changes";
  const lines = [
    COMMENT_MARKER,
    "## Message advisories",
    "",
    `${count === 1 ? "1 advisory" : `${count} advisories`} on ${subject}, from the message criteria in \`docs/pack-format.md\` ("Message"). They fail nothing: fix the message, or say in review why it stays.`,
  ];
  for (const document of documents) {
    lines.push("", `**\`${document.file}\`**`, ...document.advisories.map(advisory => `- ${escaped(advisory)}`));
  }
  return `${lines.join("\n")}\n`;
}

function main(): void {
  const { values } = parseArgs({ options: { base: { type: "string" }, all: { type: "boolean" }, out: { type: "string" } } });
  const files = values.all === true ? allRuleDocuments(RERAIL_ROOT) : changedRuleDocuments(RERAIL_ROOT, values.base ?? "HEAD^1");
  const markdown = renderAdvisories(documentAdvisories(RERAIL_ROOT, files), values.all === true);
  if (values.out === undefined) process.stdout.write(markdown);
  else writeFileSync(values.out, markdown);
}

if (process.argv[1] !== undefined && import.meta.url === `file://${process.argv[1]}`) main();
