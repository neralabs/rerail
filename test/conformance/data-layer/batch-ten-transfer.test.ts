import { readFileSync } from "node:fs";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { afterAll, expect, it } from "vitest";
import { runDataLayerCommand } from "../../../src/cli/index.js";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";
import { transfer, authoredBans, nativeProbe, nativeValues } from "../../../conformance/data-layer/transfer.js";
import { TenReplay, canonical, type TenRecord } from "../../../conformance/data-layer/batch-ten.js";

const execute = promisify(execFile);
const replay = new TenReplay();
afterAll(() => replay.close());
const scopes = new Map(transfer.scopes.map(scope => {
  const document = JSON.parse(readFileSync(`packs/data-layer/rules/${scope.rule}.json`, "utf8"));
  return [scope.rule, loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(document)), "json", capabilitiesJson())];
}));

for (const [sourceId, original] of Object.entries(transfer.cases)) {
  const contracts = new Set(original.assertions.map(assertion => assertion.contract).filter(Boolean));
  if (!contracts.size) continue;
  it(`transfers upstream assertions: ${sourceId}`, async () => {
    if (contracts.has("constants")) {
      const actual = authoredBans();
      expect([...actual.keys()].sort()).toEqual(transfer.bannedGroups.map(group => group.path).sort());
      for (const group of transfer.bannedGroups) {
        const symbols = actual.get(group.path)!;
        expect(symbols).toEqual(group.symbols);
        expect(symbols.length).toBeGreaterThan(0);
        expect(new Set(symbols).size).toBe(symbols.length);
        for (const symbol of symbols) expect(symbol).toMatch(/^[A-Za-z_]\w*$/);
      }
      expect(transfer.bannedGroups[0]!.symbols.filter(symbol => actual.get(transfer.bannedGroups[1]!.path)!.includes(symbol))).toEqual([]);
    }
    if (contracts.has("scope")) {
      for (const group of transfer.scopes) for (const path of group.paths) {
        expect(path.startsWith("/")).toBe(false);
        expect(path.split("/")).not.toContain("..");
        expect(path.endsWith(".py")).toBe(true);
        expect(scopes.get(group.rule)!.inScope(path), path).toBe(true);
        for (const outside of [path + ".bak", "elsewhere/" + path]) expect(scopes.get(group.rule)!.inScope(outside), outside).toBe(false);
      }
    }
    if (contracts.has("values")) {
      // The fields() assertion inspects a type. Exercise that schema with an
      // original constructor example instead of invoking Python reflection.
      const values = original.values.length ? original.values : Object.values(transfer.cases).flatMap(value => value.values).slice(0, 1);
      expect(values.length).toBeGreaterThan(0);
      const probe = await nativeProbe(values);
      try {
        for (const mode of ["snapshot", "workspace"] as const) {
          const actual = nativeValues(await probe.run(mode));
          expect(actual).toEqual(values);
          for (const value of actual) {
            expect(Object.keys(value).sort()).toEqual(["code", "file_path", "line_number", "message"]);
            expect(typeof value.code).toBe("string"); expect(typeof value.file_path).toBe("string");
            expect(Number.isInteger(value.line_number)).toBe(true); expect(typeof value.message).toBe("string");
          }
          if (original.source.includes("a != b")) expect(actual[0]).not.toEqual(actual[1]);
          if (original.source.includes("a == b")) expect(actual[0]).toEqual(actual[1]);
        }
      } finally { await probe.close(); }
    }
    if (contracts.has("command")) {
      const observed = original.command!;
      const probe = await nativeProbe(observed.findings);
      try {
        let output = "";
        const stdout = { write(chunk: string) { output += chunk; return true; } };
        if (observed.arguments.some(arg => arg.startsWith("--unknown") || arg === "--nope")) {
          await expect(runDataLayerCommand(observed.arguments, stdout)).rejects.toThrow("Invalid data-layer option");
          const child = await execute(process.execPath, ["--import", "tsx", resolve("src/cli/cli.ts"), "data-layer", ...observed.arguments],
            { env: { ...process.env, RERAIL_HOME: join(probe.root, "home") }, timeout: 30_000 }).then(() => 0, error => error.code);
          expect(typeof child).toBe("number"); expect(child).not.toBe(0); expect(output).toBe("");
        } else if (observed.arguments.includes("--help")) {
          expect(await runDataLayerCommand(observed.arguments, stdout)).toBe(observed.exitCode);
          expect(output).toContain("--check");
        } else {
          expect(await runDataLayerCommand([...probe.arguments, ...observed.arguments], stdout)).toBe(observed.exitCode);
          expect(output).toBe(observed.stdout.replaceAll("graph lazy SQL plan", "data-layer").replace("Graph lazy SQL intent plan linter", "Data-layer linter"));
          output = "";
          expect(await runDataLayerCommand([...probe.arguments, ...observed.arguments, "--json"], stdout)).toBe(observed.exitCode);
          expect(nativeValues(JSON.parse(output))).toEqual(observed.findings);
        }
      } finally { await probe.close(); }
    }
  }, 45_000);
}

for (const record of transfer.detectorCases as (TenRecord & { symbol: string; variant: string })[]) {
  it(`executes transferred vocabulary ${record.test} through both native APIs`, async () => {
    for (const mode of ["snapshot", "workspace"] as const)
      expect(await replay.run({ cases: {}, records: [], sources: {} }, record, mode, true)).toEqual(canonical(record.expected));
  }, 45_000);
  if (record.variant === "positive") it(`rejects removal of transferred symbol ${record.symbol}`, async () => {
    const directory = await mkdtemp(join(tmpdir(), "rerail-removed-ban-"));
    const name = "data-layer-graph-lazy-sql-plan.json";
    const before = readFileSync(`packs/data-layer/rules/${name}`, "utf8");
    const after = before.replaceAll(`'${record.symbol}'`, "'__removed_symbol__'");
    expect(after).not.toBe(before);
    await writeFile(join(directory, name), after);
    const disabled = new TenReplay(directory);
    try { expect(await disabled.run({ cases: {}, records: [], sources: {} }, record, "snapshot", true)).not.toEqual(canonical(record.expected)); }
    finally { disabled.close(); await rm(directory, { recursive: true, force: true }); }
  }, 45_000);
}

it("preserves native command paths across working directories", async () => {
  const values = [{ code: "GLSP-099", file_path: "some/file.py", line_number: 42, message: "my-message" }];
  const probe = await nativeProbe(values);
  const other = await mkdtemp(join(tmpdir(), "rerail-other-cwd-"));
  try {
    const entry = resolve("src/cli/cli.ts"), loader = import.meta.resolve("tsx");
    for (const cwd of [probe.root, other]) {
      const result = await execute(process.execPath, ["--import", loader, entry, "data-layer", ...probe.arguments, "--json"],
        { cwd, env: { ...process.env, RERAIL_HOME: join(probe.root, "home") }, timeout: 30_000 });
      expect(nativeValues(JSON.parse(result.stdout))).toEqual(values);
    }
  } finally { await probe.close(); await rm(other, { recursive: true, force: true }); }
}, 60_000);

it("rejects disabled native transport of the original CLI positives", async () => {
  const original = Object.values(transfer.cases).find(value => value.command?.findings.length)!.command!;
  const probe = await nativeProbe(original.findings);
  try {
    const path = join(probe.pack, "rules/data-layer-transfer-probe.json");
    const document = JSON.parse(readFileSync(path, "utf8"));
    for (const clause of document.clauses) clause.forbid.regex.pattern = "\\b\\B";
    await writeFile(path, JSON.stringify(document));
    let output = "";
    expect(await runDataLayerCommand([...probe.arguments, "--check"], { write(chunk: string) { output += chunk; return true; } })).toBe(0);
    expect(output).not.toContain(original.findings[0]!.code);
    expect(output).not.toBe(original.stdout);
  } finally { await probe.close(); }
});

it("transfers command help without loading a repository", async () => {
  let output = "";
  expect(await runDataLayerCommand(["--help"], { write(chunk: string) { output += chunk; return true; } })).toBe(0);
  expect(output).toContain("--check");
  expect(output).toContain("--json");
});

it("requires native evidence for every transferable upstream assertion", () => {
  const receipt = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-native-assertions.json", "utf8"));
  expect(receipt.transferableAssertions).toBeDefined();
  expect(receipt.transferableAssertions.sourceCases).toBe(57);
  expect(receipt.transferableAssertions.missing).toEqual([]);
});
