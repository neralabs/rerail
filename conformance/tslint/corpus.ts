import { readdir, readFile } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";
import { IGNORED_DIRECTORIES, languageFor } from "../../src/executable-policy-enforcer.js";
import { loadNativeBinding, type NativeCompiledPolicyDsl } from "../../src/native.js";
import { REPOSITORY_ROOT } from "./fixtures.js";
import { applyToWorkspace, capabilitiesJson, compileDocument, findPort, INFORMATIONAL_FAILURE_CODES, listPortedRules } from "./runner.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

type CorpusFile = { language: "typescript" | "tsx" | "javascript"; path: string; source: string };

export interface EditTiming {
  which: "median-size" | "largest";
  path: string;
  bytes: number;
  medianMs: number;
  runsMs: number[];
}

export interface CorpusTiming {
  directory: string;
  rules: string[];
  /** Ported rules left out because their document does not compile. */
  compileErrors: string[];
  files: number;
  bytes: number;
  compileMs: number;
  /** New workspace, every ported rule added, whole corpus applied once. */
  coldMedianMs: number;
  coldRunsMs: number[];
  /** One-file edits applied to a loaded workspace (the production hot path), on the median-size and the largest file. */
  edits: EditTiming[];
  findings: number;
  /** Failures other than `source-partial`. */
  failures: number;
  failedPaths: string[];
  /** Files with syntax errors that AST checks ran around (`source-partial`). */
  partialPaths: string[];
}

async function collect(directory: string, root: string, files: CorpusFile[]): Promise<void> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!IGNORED_DIRECTORIES.has(entry.name)) await collect(path, root, files);
      continue;
    }
    const language = languageFor(path);
    if (language !== "typescript" && language !== "tsx" && language !== "javascript") continue;
    files.push({ language, path: relative(root, path).split(sep).join("/"), source: await readFile(path, "utf8") });
  }
}

const median = (values: number[]): number => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)] ?? 0;

interface WorkspaceEvaluation {
  evaluation: { findings: unknown[]; failures: Array<{ code: string; path?: string }> };
}

/**
 * Every ported rule's default document compiled and added to ONE workspace, as the enforcer's `#prepareWorkspace` does
 * with the appropriate TypeScript, TSX, or JavaScript grammar under `directory` (relative to the repository root).
 */
export async function timeCorpus(directoryArgument: string, runs: number): Promise<CorpusTiming> {
  const directory = resolve(REPOSITORY_ROOT, directoryArgument);
  if (!directory.startsWith(REPOSITORY_ROOT + sep)) throw new Error(`${directory} is outside the repository checkout`);
  const files: CorpusFile[] = [];
  await collect(directory, REPOSITORY_ROOT, files);
  files.sort((a, b) => a.path.localeCompare(b.path));
  const bytes = files.reduce((total, file) => total + Buffer.byteLength(file.source), 0);

  const rules = listPortedRules();
  const binding = loadNativeBinding();
  capabilitiesJson();
  const compileStarted = performance.now();
  const included: string[] = [];
  const excluded: string[] = [];
  const compiled: NativeCompiledPolicyDsl[] = rules.flatMap(rule => {
    const document = findPort(rule).document;
    if (document === undefined) return [];
    try {
      const policy = compileDocument(document);
      included.push(rule);
      return [policy];
    } catch {
      excluded.push(rule);
      return [];
    }
  });
  const compileMs = performance.now() - compileStarted;

  const load = JSON.stringify({ revision: 1, files, removed: [] });
  const coldRunsMs: number[] = [];
  let last: WorkspaceEvaluation | undefined;
  for (let run = 0; run < runs; run += 1) {
    const workspace = new binding.PolicyDslWorkspace();
    for (const policy of compiled) workspace.addPolicy(policy);
    const started = performance.now();
    last = JSON.parse(await workspace.applyAsync(load)) as WorkspaceEvaluation;
    coldRunsMs.push(performance.now() - started);
    workspace.close();
  }

  const bySize = [...files].sort((a, b) => a.source.length - b.source.length);
  const targets: Array<[EditTiming["which"], CorpusFile | undefined]> = [
    ["median-size", bySize[Math.floor(bySize.length / 2)]],
    ["largest", bySize[bySize.length - 1]],
  ];
  const edits: EditTiming[] = [];
  let revision = 1;
  const workspace = new binding.PolicyDslWorkspace();
  try {
    for (const policy of compiled) workspace.addPolicy(policy);
    await workspace.applyAsync(JSON.stringify({ revision, files, removed: [] }));
    for (const [which, target] of targets) {
      if (target === undefined) continue;
      const runsMs: number[] = [];
      for (let run = 0; run < runs; run += 1) {
        revision += 1;
        const edit = { ...target, source: `${target.source}\n// rerail-conformance edit ${revision}\n` };
        const payload = JSON.stringify({ revision, files: [edit], removed: [] });
        const started = performance.now();
        await applyToWorkspace(workspace, payload);
        runsMs.push(performance.now() - started);
      }
      edits.push({ which, path: target.path, bytes: Buffer.byteLength(target.source), medianMs: median(runsMs), runsMs });
    }
  } finally {
    workspace.close();
  }

  const all = last?.evaluation.failures ?? [];
  const failures = all.filter(failure => !INFORMATIONAL_FAILURE_CODES.has(failure.code));
  const paths = (list: typeof all): string[] => [...new Set(list.flatMap(failure => (failure.path === undefined ? [] : [failure.path])))].sort(compareDeterministicStrings);
  return {
    directory: relative(REPOSITORY_ROOT, directory).split(sep).join("/"),
    rules: included,
    compileErrors: excluded,
    files: files.length,
    bytes,
    compileMs,
    coldMedianMs: median(coldRunsMs),
    coldRunsMs,
    edits,
    findings: last?.evaluation.findings.length ?? 0,
    failures: failures.length,
    failedPaths: paths(failures),
    partialPaths: paths(all.filter(failure => INFORMATIONAL_FAILURE_CODES.has(failure.code))),
  };
}
