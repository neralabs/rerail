// `compare-findings`: native's per-file findings against the first slice's (`run --engine slice`), on the clauses the
// slice lowers.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";

const sliceRunSchema = z.object({ lowered: z.array(z.object({ rule: z.string(), clause: z.string() })) });
const nativeFindingsSchema = z.object({
  violations: z.array(z.object({ ruleId: z.string().optional(), policyId: z.string().optional(), clauseId: z.string(), path: z.string(), code: z.string(), span: z.object({ startLine: z.number() }) })),
  failures: z.array(z.object({ ruleId: z.string().optional(), clauseId: z.string(), path: z.string().optional(), code: z.string() })),
});
const sliceFindingsSchema = z.object({
  findings: z.array(z.object({ rule: z.string(), code: z.string(), path: z.string(), line: z.number() })),
  unknown: z.array(z.object({ rule: z.string(), path: z.string(), reason: z.string().optional() })),
});

/** A rule id with every leading `data-layer-` stripped (`data-layer-data-layer-compat-aliases` is `compat-aliases`), on both sides. */
const ruleKey = (rule: string) => rule.replace(/^(data-layer-)+/, "");

/**
 * `compare-findings <native findings dir> <slice findings dir> <slice.json>`: per workspace (`<name>.scan.json`, and
 * `<name>.edits.json` when both have it), native's violations of the clauses the slice lowers (slice.json's `lowered`)
 * against the slice's finding keys, as sets of `[rule, code, path, line]`: the slice keeps one key per (rule, code,
 * document, line), where native may report several violations. Both count lines from 0 (the facts daemon counted from
 * 1). Prints one row per file compared with the (rule, code)s that differ and up to 5 differing keys each way, and the
 * (rule, path)s the slice leaves `Unknown` where no lowered clause of that rule failed on that path in native; returns
 * whether every workspace's keys are equal.
 */
export function compareFindings(nativeDir: string, sliceDir: string, sliceJson: string): boolean {
  const run = sliceRunSchema.parse(JSON.parse(readFileSync(sliceJson, "utf8")));
  const lowered = new Set(run.lowered.map(({ rule, clause }) => `${ruleKey(rule)}\u0000${clause}`));
  let equal = true;
  for (const file of readdirSync(sliceDir).filter(name => name.endsWith(".json")).sort()) {
    const nativePath = join(nativeDir, file);
    if (!existsSync(nativePath)) continue;
    const nativeFindings = nativeFindingsSchema.parse(JSON.parse(readFileSync(nativePath, "utf8")));
    const violations = nativeFindings.violations.filter(v => lowered.has(`${ruleKey(v.ruleId ?? v.policyId ?? "")}\u0000${v.clauseId}`));
    const native = new Set(violations.map(v => JSON.stringify([ruleKey(v.ruleId ?? v.policyId ?? ""), v.code, v.path, v.span.startLine])));
    const slice = sliceFindingsSchema.parse(JSON.parse(readFileSync(join(sliceDir, file), "utf8")));
    const ours = new Set(slice.findings.map(f => JSON.stringify([ruleKey(f.rule), f.code, f.path, f.line])));
    const onlyNative = [...native].filter(key => !ours.has(key));
    const onlySlice = [...ours].filter(key => !native.has(key));
    const failures = nativeFindings.failures.filter(f => lowered.has(`${ruleKey(f.ruleId ?? "")}\u0000${f.clauseId}`));
    const failed = new Set(failures.map(f => `${ruleKey(f.ruleId ?? "")}\u0000${f.path ?? ""}`));
    const unexplained = slice.unknown.filter(u => !failed.has(`${ruleKey(u.rule)}\u0000${u.path}`));
    equal &&= onlyNative.length === 0 && onlySlice.length === 0;
    const byClause = new Map<string, [number, number]>();
    for (const [key, side] of [...onlyNative.map(key => [key, 0] as const), ...onlySlice.map(key => [key, 1] as const)]) {
      const [rule, code] = JSON.parse(key) as [string, string];
      const counts = byClause.get(`${rule} ${code}`) ?? [0, 0];
      counts[side] += 1;
      byClause.set(`${rule} ${code}`, counts);
    }
    process.stdout.write(
      `${file}: native ${violations.length} violations (${native.size} keys), slice ${ours.size} keys, only native ${onlyNative.length}, only slice ${onlySlice.length}; ` +
        `slice unknown ${slice.unknown.length} (${unexplained.length} where native did not fail)\n`,
    );
    for (const [clause, [left, right]] of [...byClause].sort()) process.stdout.write(`  differ ${clause}: only native ${left}, only slice ${right}\n`);
    for (const key of [...onlyNative.slice(0, 5).map(k => `  native-only ${k}`), ...onlySlice.slice(0, 5).map(k => `  slice-only ${k}`)]) process.stdout.write(`${key}\n`);
    for (const u of unexplained.slice(0, 5)) process.stdout.write(`  unknown ${u.rule} in ${u.path}: ${u.reason ?? "(no reason)"}, native did not fail\n`);
  }
  return equal;
}
