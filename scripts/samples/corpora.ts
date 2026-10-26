/**
 * What the builder reads of a pack: its conformance manifest's rules (to key labels by pack rule id, D4) and corpora,
 * the pinned corpus checkouts, the corpus files the conformance check scored, and the context files beside a file.
 */
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join, matchesGlob, posix, resolve } from "node:path";

import { checkoutPinned, readRerailCorpora, type RerailCorpus } from "../../conformance/port-readiness/corpus.js";
import type { CommandRunner } from "../../conformance/port-readiness/process.js";
import { CONTEXT_FILES } from "../../conformance/port-readiness/recording/corpus-answers.js";
import { CODE_RULE_DIALECTS, IGNORED_DIRECTORIES, languageFor, loadNativeBinding } from "../../src/engine/index.js";
import { byText, type ContextFile, type ToolPack } from "./samples.js";

export interface PackRule {
  id: string;
  /** The rule document, relative to the pack directory. */
  policy: string;
  upstream: { tool: string; rule: string };
}

export interface PackConformance {
  pack: string;
  family: string;
  rules: PackRule[];
  corpora: RerailCorpus[];
}

/**
 * A pack's rules and corpora. The rules are its conformance manifest's alone: the conformance check judges samples
 * for the rules of `packs/<pack>/conformance.json`, so a label key from another manifest (node's `candidates/`) would
 * name no rule it loads.
 */
export async function readConformance(root: string, pack: string): Promise<PackConformance> {
  const manifest = JSON.parse(readFileSync(join(root, "packs", pack, "conformance.json"), "utf8")) as { family: string; rules: PackRule[] };
  return { pack, family: manifest.family, rules: manifest.rules, corpora: await readRerailCorpora(root, pack) };
}

/** The pack rule ids porting one upstream rule of one tool (D4: each gets the upstream rule's labels). */
export function packRuleIds(conformance: PackConformance, tool: string, upstreamRule: string): string[] {
  return conformance.rules.filter((rule) => rule.upstream.tool === tool && rule.upstream.rule === upstreamRule).map((rule) => rule.id).sort(byText);
}

/** `owner/name` of a corpus's GitHub repository. */
export const corpusRepo = (corpus: RerailCorpus): string => corpus.repository.replace(/^https:\/\/github\.com\//u, "");

/** The project files a file of an ECMAScript family reads beside it (the conformance check sent them with corpora). */
const MANIFEST_PATTERNS = ["package.json", "tsconfig*.json", "jsconfig*.json"];

/**
 * The corpus checked out at its pinned commit: its scored paths and every project manifest, in the port-readiness
 * cache's sparse checkout (shared with the gates' golden fetch, widened, never narrowed).
 */
export async function corpusCheckout(cacheRoot: string, corpus: RerailCorpus, runner: CommandRunner): Promise<string> {
  const patterns = [...corpus.paths.map((path) => (path === "." ? "/*" : `/${path}`)), ...MANIFEST_PATTERNS];
  return checkoutPinned(corpus.repository, corpus.commit, resolve(cacheRoot, "corpus", "rerail", corpus.name, corpus.commit), patterns, runner);
}

function walk(root: string, relative: string, files: string[]): void {
  const absolute = join(root, relative);
  const entry = lstatSync(absolute, { throwIfNoEntry: false });
  if (entry === undefined) return;
  if (entry.isFile()) {
    files.push(relative);
    return;
  }
  if (!entry.isDirectory()) return;
  for (const child of readdirSync(absolute, { withFileTypes: true })) {
    if (child.isDirectory() && IGNORED_DIRECTORIES.has(child.name)) continue;
    walk(root, relative === "." ? child.name : `${relative}/${child.name}`, files);
  }
}

/**
 * The corpus files the conformance check's corpus tier scored, which a corpus recording speaks for: every file under
 * the corpus's paths (ignored directories aside) of a language of the pack's family, not excluded by a glob nor skipped
 * by the recording. Sorted.
 */
export function corpusFiles(checkout: string, corpus: RerailCorpus, family: string, skipped: ReadonlySet<string>): string[] {
  const languages = new Set(Object.values(CODE_RULE_DIALECTS).filter((dialect) => dialect.family === family).map((dialect) => dialect.label as string));
  const found: string[] = [];
  for (const path of corpus.paths) walk(checkout, path, found);
  return [...new Set(found)]
    .filter((path) => {
      const language = languageFor(path);
      return language !== undefined && languages.has(language) && !skipped.has(path) && !corpus.exclude.some((glob) => matchesGlob(path, glob));
    })
    .sort(byText);
}

/** moduleConfigFiles by checkout and folder: every file of a package asks for the same chain. */
const configurationChains = new Map<string, string[]>();

/**
 * The files a sample needs beside it, read through `source` (a checkout, or a task's fetched files): for Ruff the
 * `__init__.py`/`__init__.pyi` above it (CONTEXT_FILES); for a language whose files read package.json, from a
 * checkout, each package.json at or above it and its packages' project configuration chain (moduleConfigFiles), as
 * the conformance check sent them with a corpus.
 */
export function contextFiles(toolPack: ToolPack, path: string, source: { read(path: string): Buffer | undefined; checkout?: string }): ContextFile[] {
  const found = new Map<string, Buffer>();
  // Ruff resolves a file's package, so its module name, from the `__init__.py` files above it.
  if (toolPack.tool === "ruff") {
    for (const marker of CONTEXT_FILES.ruff(path)) {
      const bytes = source.read(marker);
      if (bytes !== undefined) found.set(marker, bytes);
    }
  }
  const language = languageFor(path);
  const checkout = source.checkout;
  if (checkout !== undefined && language !== undefined && loadNativeBinding().readsPackageManifests(language)) {
    for (let folder = posix.dirname(path); ; folder = posix.dirname(folder)) {
      const manifest = folder === "." ? "package.json" : `${folder}/package.json`;
      const bytes = source.read(manifest);
      if (bytes !== undefined) {
        found.set(manifest, bytes);
        const key = `${checkout}\0${folder}`;
        if (!configurationChains.has(key)) configurationChains.set(key, loadNativeBinding().moduleConfigFiles(checkout, folder === "." ? "" : folder));
        for (const configuration of configurationChains.get(key)!) {
          if (configuration.startsWith("..")) continue;
          const text = source.read(configuration);
          if (text !== undefined) found.set(configuration, text);
        }
      }
      if (folder === ".") break;
    }
  }
  found.delete(path);
  return [...found].map(([file, bytes]) => ({ path: file, bytes })).sort((a, b) => byText(a.path, b.path));
}

/** Reads regular files of a checkout, by repository-relative path; anything else is absent. */
export function checkoutReader(checkout: string): { read(path: string): Buffer | undefined; checkout: string } {
  return {
    checkout,
    read(path) {
      const absolute = join(checkout, path);
      return lstatSync(absolute, { throwIfNoEntry: false })?.isFile() === true ? readFileSync(absolute) : undefined;
    },
  };
}
