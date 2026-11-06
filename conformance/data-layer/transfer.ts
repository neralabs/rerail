/** Native result transport for the upstream tests that construct synthetic violations. */
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { runDataLayerAudit, type DataLayerAuditResult } from "../../src/cli/index.js";

export type Value = { code: string; file_path: string; line_number: number; message: string };
export type TransferCase = { source: string; sha256: string; assertions: { source: string; contract: string | null; reason: string }[]; values: Value[];
  command?: { arguments: string[]; findings: Value[]; stdout: string; stderr: string; exitCode: number; raised: boolean } };
export const transfer = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-transferable-assertions.json", "utf8")) as {
  cases: Record<string, TransferCase>; bannedGroups: { name: string; path: string; symbols: string[] }[];
  detectorCases: import("./batch-ten.js").TenRecord[]; scopes: { rule: string; paths: string[] }[];
};

export function authoredBans(): Map<string, string[]> {
  const authored = JSON.parse(readFileSync("packs/data-layer/rules/data-layer-graph-lazy-sql-plan.json", "utf8"));
  const result = new Map<string, string[]>();
  for (const clause of authored.clauses) {
    if (!clause.relational) continue;
    const queries = new Set<string>();
    function visit(value: unknown): void { if (typeof value === "string") queries.add(value); else if (value && typeof value === "object") for (const child of Object.values(value)) visit(child); }
    visit(clause.relational.query);
    for (const query of queries) for (const match of query.matchAll(/\bn\.__path='([^']+)'(?:(?! UNION ALL )[\s\S])*?\bn\.name IN \(((?:'[^']*'(?:,\s*)?)+)\)/g)) {
      const symbols = [...match[2]!.matchAll(/'([^']*)'/g)].map(token => token[1]!).sort();
      const previous = result.get(match[1]!);
      if (previous && JSON.stringify(previous) !== JSON.stringify(symbols)) throw new Error("Conflicting authored banned vocabularies");
      result.set(match[1]!, symbols);
    }
  }
  return result;
}

export async function nativeProbe(values: Value[]) {
  const root = await mkdtemp(join(tmpdir(), "rerail-transfer-"));
  const pack = join(root, "pack"), snapshotPath = join(root, "snapshot.json"), id = "data-layer-transfer-probe";
  const lines = new Map<string, string[]>();
  for (const [index, value] of values.entries()) {
    const source = lines.get(value.file_path) ?? [];
    while (source.length < value.line_number) source.push("");
    source[value.line_number - 1] += ` # TRANSFER_${index}`;
    lines.set(value.file_path, source);
  }
  const files = [...lines].map(([path, source]) => ({ path, source: source.join("\n") + "\n", language: "python" }));
  if (!files.length) files.push({ path: "clean.py", source: "pass\n", language: "python" });
  const clauses = values.map((value, index) => ({ id: `value-${index}`, forbid: { regex: { source: "file.content", pattern: `\\bTRANSFER_${index}\\b` } },
    violation: { code: value.code, message: value.message, anchor: "witness.span" } }));
  if (!clauses.length) clauses.push({ id: "clean", forbid: { regex: { source: "file.content", pattern: "\\b\\B" } }, violation: { code: "CLEAN", message: "No findings", anchor: "witness.span" } });
  const document = { format: "rerail.policy/v1", id, revision: 1, mode: "block", scope: { include: ["**/*"], exclude: [] }, clauses };
  const manifest = JSON.parse(readFileSync("packs/data-layer/pack.json", "utf8"));
  manifest.rules = [{ id, document: `rules/${id}.json`, title: "Transfer contract", group: "Data-layer architecture" }];
  await mkdir(join(pack, "rules"), { recursive: true });
  await writeFile(join(pack, "pack.json"), JSON.stringify(manifest));
  await writeFile(join(pack, `rules/${id}.json`), JSON.stringify(document));
  const snapshot = { format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] };
  await writeFile(snapshotPath, JSON.stringify(snapshot));
  for (const file of files) { const path = join(root, file.path); await mkdir(dirname(path), { recursive: true }); await writeFile(path, file.source); }
  return {
    root, pack, snapshot, snapshotPath,
    arguments: ["--repo", root, "--pack", pack, "--snapshot", snapshotPath, "--rules", id],
    run: (mode: "snapshot" | "workspace") => runDataLayerAudit({ repositoryRoot: root, packDirectory: pack, ruleIds: [id], snapshot, mode }),
    close: () => rm(root, { recursive: true, force: true }),
  };
}

export function nativeValues(result: DataLayerAuditResult): Value[] {
  if (result.failures.length) throw new Error(JSON.stringify(result.failures));
  const rows = result.violations as (DataLayerAuditResult["violations"][number] & { clauseId: string })[];
  return [...rows].sort((a, b) => Number(a.clauseId.slice(6)) - Number(b.clauseId.slice(6))).map(row => {
    if (!row.span) throw new Error("Native diagnostic has no source span");
    return { code: row.code, file_path: row.path, line_number: row.span.startLine + 1, message: row.message };
  });
}
