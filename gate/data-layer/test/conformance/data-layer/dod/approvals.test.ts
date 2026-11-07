import { readFileSync } from "node:fs";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { gunzipSync } from "node:zlib";
import { expect, it } from "vitest";
import { decodeSchemaApprovalSignature, runDataLayerAudit } from "../../../../src/cli/index.js";
const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-dod-contracts.json", "utf8"));
type Case = { id: string; entries: string[]; signature: string; trustedPublicKey: string; files: { path: string; source: string; language: string }[]; expected: unknown[] };
const canonical = (value: unknown[]) => value.map(row => JSON.stringify(row)).sort();
it("matches original Python Base64 decoding for malformed and padded approval signatures", () => {
  const fixture = JSON.parse(gunzipSync(readFileSync("fixtures/host/data-layer-port/batch-10-base64-decoding.json.gz")).toString("utf8"));
  expect(fixture.cases).toHaveLength(68008);
  for (const row of fixture.cases as { text: string; expected: string | null }[]) {
    let actual = null;
    try { actual = decodeSchemaApprovalSignature(row.text).toString("hex"); } catch { /* Preserve original decoding failures. */ }
    expect(actual, JSON.stringify(row.text)).toBe(row.expected);
  }
});
for (const record of oracle.approvalCases as Case[]) for (const mode of ["snapshot", "workspace"] as const)
  it(`preserves original approval ${record.id} through ${mode}`, async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-approval-contract-"));
    try {
      const pack = join(root, "pack"), id = "data-layer-schema-introspection-data-paths";
      await mkdir(join(pack, "runtime"), { recursive: true }); await mkdir(join(pack, "rules"));
      const manifest = JSON.parse(readFileSync("packs/data-layer/pack.json", "utf8"));
      manifest.rules = manifest.rules.filter((rule: { id: string }) => rule.id === id);
      await writeFile(join(pack, "pack.json"), JSON.stringify(manifest));
      await writeFile(join(pack, `rules/${id}.json`), readFileSync(`packs/data-layer/rules/${id}.json`));
      // A fixture trust root never changes the trusted key in the shipped pack.
      await writeFile(join(pack, "runtime/signing-public-key.json"), JSON.stringify({ publicKeyHex: record.trustedPublicKey }));
      const options = { repositoryRoot: root, packDirectory: pack, ruleIds: [id], mode,
        snapshot: { format: "rerail.repository-snapshot/v1", revision: 0, files: record.files },
        schemaApprovals: { entries: record.entries, signature: record.signature } };
      const result = await runDataLayerAudit(options);
      expect(result.failures).toEqual([]);
      expect(canonical(result.violations.map(row => ({ code: row.code, path: row.path, line: row.span!.startLine + 1, message: row.message })))).toEqual(canonical(record.expected));
      if (record.id === "approved-one") {
        expect(oracle.approvalEncodingCases).toHaveLength(8);
        for (const encoded of oracle.approvalEncodingCases as Case[]) {
          const checked = await runDataLayerAudit({ ...options, schemaApprovals: { entries: encoded.entries, signature: encoded.signature } });
          expect(checked.failures).toEqual([]);
          expect(canonical(checked.violations.map(row => ({ code: row.code, path: row.path,
            line: row.span!.startLine + 1, message: row.message }))), encoded.id).toEqual(canonical(encoded.expected));
        }
      }
      if (record.id === "invalid-internal") {
        const empty = await runDataLayerAudit({ ...options, snapshot: { ...options.snapshot, files: [] } });
        expect(empty.failures).toEqual([]);
        expect(canonical(empty.violations.map(row => ({ code: row.code, path: row.path, line: row.span!.startLine + 1, message: row.message })))).toEqual(canonical(record.expected));
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
