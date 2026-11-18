import { readFileSync, existsSync } from "node:fs";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { generateKeyPairSync } from "node:crypto";
import { expect, it } from "vitest";
import { runDataLayerAudit, runDataLayerCommand, schemaApprovalMessage } from "../../../../src/cli/index.js";
const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-dod-contracts.json", "utf8"));
const python = [".rerail/data-layer-batch-eight/ci-venv/bin/python", `${process.env.RUNNER_TEMP}/batch-eight-venv/bin/python`].map(p => resolve(p)).find(existsSync);
it("preserves the original signing canonicalization and deployed trust root", () => {
  for (const record of oracle.approvalCases) expect(schemaApprovalMessage(record.entries).toString()).toBe(record.canonicalMessage);
  expect(JSON.parse(readFileSync("packs/data-layer/runtime/signing-public-key.json", "utf8")).publicKeyHex).toBe(oracle.deployedPublicKey);
});
it("routes original matching phases and summary-only output through the production command", async () => {
  const root = await mkdtemp(join(tmpdir(), "rerail-dod-command-"));
  try {
    await mkdir(join(root, "consumers"));
    const snapshot = join(root, "snapshot.json");
    await writeFile(snapshot, JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
      files: [{ path: "consumers/probe.py", source: "import impl.common.hidden\n", language: "python" }] }));
    const base = ["--repo", root, "--pack", "packs/data-layer", "--snapshot", snapshot];
    let output = "";
    const stream = { write(chunk: string) { output += chunk; return true; } };
    for (const phase of [1, 2, 3, 4]) {
      output = "";
      expect(await runDataLayerCommand([...base, "--rules", "data-layer-matching-architecture", "--application-root", "consumers", "--phase", String(phase), "--json", "--check"], stream)).toBe(phase === 1 ? 1 : 0);
      expect(JSON.parse(output).violations.map((r: { code: string }) => r.code)).toEqual(phase === 1 ? ["MA010"] : []);
    }
    await expect(runDataLayerCommand([...base, "--phase", "5"], stream)).rejects.toThrow(/phase/);
    await writeFile(snapshot, JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
      files: [{ path: "impl/common/l2065_plan_core/verifier.py", source: "if True:\n", language: "python" }] }));
    output = "";
    expect(await runDataLayerCommand([...base, "--rules", "data-layer-plan-optimizer-v2", "--summary-only", "--check"], stream)).toBe(1);
    expect(output).toContain("PV2-050: 1"); expect(output).not.toContain("verifier.py:");
  } finally { await rm(root, { recursive: true, force: true }); }
});
it("reads live source approvals and delegates only explicit signing to the original signer", async () => {
  if (!python) throw new Error("The original signer tests require the pinned PyNaCl environment");
  const root = await mkdtemp(join(tmpdir(), "rerail-dod-signing-"));
  try {
    const pack = join(root, "pack"), id = "data-layer-schema-introspection-data-paths";
    await mkdir(join(pack, "runtime"), { recursive: true }); await mkdir(join(pack, "rules")); await mkdir(join(root, "scripts"));
    const manifest = JSON.parse(readFileSync("packs/data-layer/pack.json", "utf8"));
    manifest.rules = manifest.rules.filter((r: { id: string }) => r.id === id);
    await writeFile(join(pack, "pack.json"), JSON.stringify(manifest));
    for (const path of [`rules/${id}.json`, "runtime/approvals.py"]) await writeFile(join(pack, path), readFileSync(`packs/data-layer/${path}`));
    const key = generateKeyPairSync("ed25519");
    const seed = key.privateKey.export({ format: "der", type: "pkcs8" }).subarray(-32).toString("hex");
    await writeFile(join(pack, "runtime/signing-public-key.json"), JSON.stringify({ publicKeyHex: key.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex") }));
    const script = join(root, "scripts/lint_schema_introspection_data_paths.py");
    // These are byte-for-byte original signing functions, executed only in an isolated fixture repository.
    const fixture = `from __future__ import annotations\nimport base64,json,os,re,sys\nfrom pathlib import Path\nfrom nacl.signing import SigningKey\nNEEDS_SIGNING='NEEDS_SIGNING'\nUNSIGNED='UNSIGNED'\nAPPROVED_USER_SCHEMA_CHECK_CALLSITES=['l2050_transaction/consumer.py:1:table_exists']\nAPPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE = NEEDS_SIGNING\n${oracle.originalSignerFunctions}\nif __name__=='__main__':\n    assert sys.argv[1:]==['--sign-allowlist']\n    raise SystemExit(0 if sign_and_update_allowlist(script_path=Path(__file__),linter_name='schema_introspection_data_paths',entries=sorted(APPROVED_USER_SCHEMA_CHECK_CALLSITES),variable_name='APPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE') else 1)\n`;
    await writeFile(script, fixture);
    const options = { repositoryRoot: root, packDirectory: pack, python, ruleIds: [id], snapshot: { format: "rerail.repository-snapshot/v1", revision: 0,
      files: [{ path: "l2050_transaction/consumer.py", source: "table_exists()\n", language: "python" }] } };
    expect((await runDataLayerAudit(options)).violations.map(r => r.code)).toEqual(["SIDP000"]);
    expect(readFileSync(script, "utf8")).toBe(fixture);
    let output = "";
    // A generated fixture credential is passed explicitly; the test never reads a deployed signing credential.
    expect(await runDataLayerCommand(["--repo", root, "--pack", pack, "--python", python, "--sign-allowlist"],
      { write(chunk: string) { output += chunk; return true; } }, { NERA_SIGNING_KEY: seed })).toBe(0);
    expect(readFileSync(script, "utf8")).not.toBe(fixture);
    for (const mode of ["snapshot", "workspace"] as const) expect((await runDataLayerAudit({ ...options, mode })).violations).toEqual([]);
    await writeFile(join(root, "approvals.json"), JSON.stringify({ entries: ["impl/bad.py:1:table_exists"], signature: "NEEDS_SIGNING" }));
    const snapshot = join(root, "snapshot.json"); await writeFile(snapshot, JSON.stringify(options.snapshot)); output = "";
    expect(await runDataLayerCommand(["--repo", root, "--pack", pack, "--snapshot", snapshot, "--rules", id,
      "--schema-approvals", join(root, "approvals.json"), "--summary", "--check"], { write(chunk: string) { output += chunk; return true; } })).toBe(1);
    expect(output).toBe("1 schema introspection data-path violation(s)\n");
    output = "";
    expect(await runDataLayerCommand(["--repo", root, "--pack", pack, "--snapshot", snapshot, "--rules", id,
      "--schema-approvals", join(root, "approvals.json"), "--json"], { write(chunk: string) { output += chunk; return true; } })).toBe(1);
    expect(JSON.parse(output).violations.map((row: { code: string }) => row.code)).toEqual(["SIDP000"]);
    await writeFile(script, "APPROVED_USER_SCHEMA_CHECK_CALLSITES=[]\nAPPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE='UNSIGNED'\n");
    expect((await runDataLayerAudit(options)).violations.map(r => r.code)).toEqual(["SIDP001"]);
  } finally { await rm(root, { recursive: true, force: true }); }
});
