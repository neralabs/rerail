// `conformance`: each ported rule over the whole tree, then each recorded mutation overlaid in memory, compared with the
// linter's recorded findings as multisets of (code, path, line).
import { compareStrings, findingKey, RuleWorkspace, type Engine, type FindingKey, type PortedRule, type SourceFile } from "./engine.js";
import { modulePath, type Repository } from "./corpus.js";
import { compareFindings, expectedAfter, judgeMutation, loadLinterOracle, loadUnsupported, subtract, unknownSidecarMutations, type Comparison } from "./oracle.js";

export interface ConformanceOptions {
  engine: Engine;
  repository: Repository;
  rules: PortedRule[];
  /** One directory per linter: its port and the linter's recorded oracle. */
  oracles: string;
  /** Differing findings printed per list; the rest are counted. */
  show: number;
}

function printDifferences(label: string, findings: readonly FindingKey[], show: number): void {
  if (findings.length === 0) return;
  console.log(`      ${label} (${findings.length}):`);
  for (const finding of findings.slice(0, show)) console.log(`        ${finding.code} ${finding.path}:${finding.line}`);
  if (findings.length > show) console.log(`        … ${findings.length - show} more`);
}

function countByCode(findings: readonly FindingKey[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const finding of findings) counts[finding.code] = (counts[finding.code] ?? 0) + 1;
  return counts;
}

/**
 * Evaluates every rule that has an oracle over the whole repository, then each mutation overlaid in memory, and
 * compares with the linter's recorded findings. Returns the process exit code.
 */
export async function runConformance(options: ConformanceOptions): Promise<number> {
  const { engine, repository, show } = options;
  const byPath = new Map(repository.files.map(file => [file.path, file]));
  const failedCases: string[] = [];
  let cases = 0;
  let checked = 0;
  const record = (name: string, passed: boolean) => {
    cases += 1;
    if (!passed) failedCases.push(name);
  };
  for (const rule of options.rules) {
    const oracle = loadLinterOracle(options.oracles, rule.linter);
    if (oracle === undefined) {
      console.log(`${rule.linter}: no oracle under ${options.oracles}; skipped`);
      continue;
    }
    if (repository.git.commit !== oracle.dataLayerCommit) {
      throw new Error(`${repository.root} is at ${repository.git.commit}, but the ${rule.linter} oracle was recorded at ${oracle.dataLayerCommit}`);
    }
    if (repository.git.changes.length > 0) {
      throw new Error(`${repository.root} has working-tree changes; the oracle describes the clean commit:\n${repository.git.changes.join("\n")}`);
    }
    checked += 1;
    const unsupported = loadUnsupported(options.oracles, rule.linter);
    const excluded = new Set(unsupported.filter(entry => entry.mismatch === undefined).map(entry => entry.code));
    const partial = unsupported.filter(entry => entry.mismatch !== undefined);
    const compared = (findings: readonly FindingKey[]) => findings.filter(finding => !excluded.has(finding.code));
    const workspace = new RuleWorkspace(engine, [rule]);
    try {
      const accepted = repository.files.filter(file => workspace.accepts(file.path));
      const tree = await workspace.apply(accepted);
      const treeFindings = compared(tree.result.violations.map(findingKey));
      const treeComparison = compareFindings(compared(oracle.tree), treeFindings);
      // A partial gap is known when its code differs from the linter by exactly the recorded counts.
      const fp = countByCode(treeComparison.extra);
      const fn = countByCode(treeComparison.missing);
      const known = new Set(partial
        .filter(entry => (fp[entry.code] ?? 0) === entry.mismatch!.fp && (fn[entry.code] ?? 0) === entry.mismatch!.fn)
        .map(entry => entry.code));
      const unknownExtra = treeComparison.extra.filter(finding => !known.has(finding.code));
      const unknownMissing = treeComparison.missing.filter(finding => !known.has(finding.code));
      const stalePartial = partial.filter(entry => !known.has(entry.code));
      const treePassed = unknownExtra.length === 0 && unknownMissing.length === 0 && stalePartial.length === 0;
      const verdict = !treePassed ? "FAIL" : excluded.size > 0 ? "pass on supported codes only" : "pass";
      console.log(`${rule.linter}: ${accepted.length} files in scope; tree TP ${treeComparison.matched} FP ${treeComparison.extra.length} FN ${treeComparison.missing.length} — ${verdict}`);
      if (excluded.size > 0) console.log(`    unsupported (excluded, not passes): ${[...excluded].sort(compareStrings).join(", ")}`);
      for (const entry of partial) {
        console.log(`    partial ${entry.code}: recorded FP ${entry.mismatch!.fp} FN ${entry.mismatch!.fn}; observed FP ${fp[entry.code] ?? 0} FN ${fn[entry.code] ?? 0}${known.has(entry.code) ? " (known)" : " (DIFFERS from the record)"}`);
      }
      printDifferences("false positives", unknownExtra, show);
      printDifferences("false negatives", unknownMissing, show);
      if (tree.result.failures.length > 0) {
        console.log(`    engine failures (${tree.result.failures.length}): ${JSON.stringify(countByCode(tree.result.failures.map(failure => ({ code: failure.code, path: failure.path ?? "", line: 0 }))))}`);
        for (const failure of tree.result.failures.slice(0, show)) console.log(`        ${failure.code} ${failure.path ?? ""} ${failure.clauseId}: ${failure.message}`);
      }
      if (tree.result.blindFiles.length > 0) console.log(`    blind files (${tree.result.blindFiles.length}): ${tree.result.blindFiles.slice(0, show).map(file => file.path).join(", ")}`);
      const stale = unknownSidecarMutations(unsupported, oracle);
      if (stale.length > 0) console.log(`    FAIL: the sidecar records gaps for mutations the oracle does not have: ${stale.join(", ")}`);
      record(`${rule.linter}/tree`, treePassed && stale.length === 0);
      // Files the rule's scope includes that are not Python: the harness never evaluates them. Listed once per rule.
      const notEvaluated = new Set(repository.otherPaths.filter(path => rule.compiled.inScope(path)));
      for (const mutation of oracle.mutations) {
        for (const path of mutation.files.keys()) if (!path.endsWith(".py") && rule.compiled.inScope(path)) notEvaluated.add(path);
      }
      if (notEvaluated.size > 0) console.log(`    in scope but not evaluated (not Python): ${[...notEvaluated].sort(compareStrings).join(", ")}`);
      for (const mutation of oracle.mutations) {
        const overlay = [...mutation.files]
          .filter(([path]) => path.endsWith(".py") && workspace.accepts(path))
          .map(([path, source]): SourceFile => ({ path, language: "python", source, ...modulePath(engine, repository.root, path) }));
        const after = await workspace.apply(overlay);
        const comparison: Comparison = compareFindings(compared(expectedAfter(oracle.tree, mutation)), compared(after.result.violations.map(findingKey)));
        // Differences the tree already shows (known partial gaps) are not the mutation's: compare the rest.
        const extra = subtract(comparison.extra, treeComparison.extra);
        const missing = subtract(comparison.missing, treeComparison.missing);
        const judged = judgeMutation(mutation.name, extra, missing, unsupported);
        console.log(`  ${mutation.name}: +${mutation.added.length} -${mutation.removed.length} expected — ${judged.passed ? "pass" : "FAIL"}`);
        for (const residual of judged.codes) {
          const recorded = residual.recorded.fp + residual.recorded.fn > 0;
          if (!recorded && residual.matches) continue;
          const state = residual.matches ? "known gap" : recorded ? "DIFFERS from the recorded gap" : "unrecorded";
          console.log(`      ${residual.code}: FP ${residual.fp} FN ${residual.fn}; recorded FP ${residual.recorded.fp} FN ${residual.recorded.fn} (${state})`);
        }
        printDifferences("false positives", extra, show);
        printDifferences("false negatives", missing, show);
        record(`${rule.linter}/${mutation.name}`, judged.passed);
        const restored = overlay.flatMap(file => byPath.get(file.path) ?? []);
        await workspace.apply(restored, overlay.filter(file => !byPath.has(file.path)).map(file => file.path));
      }
    } finally {
      workspace.close();
    }
  }
  if (checked === 0) {
    console.log(`No rule in ${options.oracles} has an oracle`);
    return 1;
  }
  console.log(`\nconformance: ${checked} rules, ${cases} cases, ${cases - failedCases.length} passed, ${failedCases.length} failed`);
  if (failedCases.length > 0) console.log(`  failed: ${failedCases.join(", ")}`);
  return failedCases.length > 0 ? 1 : 0;
}
