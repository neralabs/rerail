// `compare-findings`: native's per-file findings against another engine's. The facts daemon that wrote the other side
// (`run --engine facts`) is not on this branch, so only the comparison is kept.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";

const helloSchema = z.object({ hello: z.object({ per_file_clauses: z.array(z.tuple([z.string(), z.string()])) }) });
const nativeFindingsSchema = z.object({
  violations: z.array(z.object({ ruleId: z.string().optional(), policyId: z.string().optional(), clauseId: z.string(), path: z.string(), code: z.string(), span: z.object({ startLine: z.number() }) })),
});
const factsFindingsSchema = z.object({ findings: z.array(z.tuple([z.string(), z.string(), z.string(), z.number(), z.string(), z.number().nullable()])) });

/**
 * `compare-findings <native findings dir> <facts findings dir> [<facts.json>]`: per workspace (`<name>.scan.json`, and
 * `<name>.edits.json` when both have it), native's violations of the clauses the facts daemon answers per file (its
 * `hello`'s `per_file_clauses`, from facts.json) against the daemon's per-file findings, as multisets of
 * `[check, clause, path, line, code]` (native's rule id `data-layer-a-b` is check `a_b`, every leading `data-layer-`
 * stripped as the daemon does: `data-layer-data-layer-compat-aliases` is `compat_aliases`). Prints one row per file
 * compared and up to 5 differences each; returns whether every workspace is equal.
 */
export function compareFindings(nativeDir: string, factsDir: string, factsJson: string): boolean {
  const run = helloSchema.parse(JSON.parse(readFileSync(factsJson, "utf8")));
  const perFile = new Set(run.hello.per_file_clauses.map(([check, clause]) => `${check}\u0000${clause}`));
  let equal = true;
  for (const file of readdirSync(factsDir).filter(name => name.endsWith(".json")).sort()) {
    const nativePath = join(nativeDir, file);
    if (!existsSync(nativePath)) continue;
    const native = nativeFindingsSchema.parse(JSON.parse(readFileSync(nativePath, "utf8"))).violations
      .map(v => [(v.ruleId ?? v.policyId ?? "").replace(/^(data-layer-)+/, "").replaceAll("-", "_"), v.clauseId, v.path, v.span.startLine + 1, v.code] as const)
      .filter(([check, clause]) => perFile.has(`${check}\u0000${clause}`))
      .map(key => JSON.stringify(key));
    const facts = factsFindingsSchema.parse(JSON.parse(readFileSync(join(factsDir, file), "utf8"))).findings
      .map(([check, clause, path, line, code]) => JSON.stringify([check, clause, path, line, code]));
    const minus = (left: string[], right: string[]) => {
      const counts = new Map<string, number>();
      for (const key of right) counts.set(key, (counts.get(key) ?? 0) + 1);
      return left.filter(key => {
        const n = counts.get(key) ?? 0;
        if (n > 0) counts.set(key, n - 1);
        return n === 0;
      });
    };
    const onlyNative = minus(native, facts);
    const onlyFacts = minus(facts, native);
    equal &&= onlyNative.length === 0 && onlyFacts.length === 0;
    process.stdout.write(`${file}: native ${native.length}, facts ${facts.length}, only native ${onlyNative.length}, only facts ${onlyFacts.length}\n`);
    for (const key of [...onlyNative.slice(0, 5).map(k => `  native-only ${k}`), ...onlyFacts.slice(0, 5).map(k => `  facts-only ${k}`)]) process.stdout.write(`${key}\n`);
  }
  return equal;
}
