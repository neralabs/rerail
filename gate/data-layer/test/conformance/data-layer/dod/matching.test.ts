import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { expect, it } from "vitest";
import { runDataLayerAudit, runDataLayerCommand } from "../../../../src/cli/index.js";
for (const mode of ["snapshot", "workspace"] as const) it(`discovers and classifies explicit matching roots through ${mode}`, async () => {
  const root = await mkdtemp(join(tmpdir(), "rerail-matching-roots-"));
  try {
    await mkdir(join(root, "consumers/tests"), { recursive: true }); await mkdir(join(root, "second"));
    await writeFile(join(root, "consumers/discovered.py"), "import impl.common.hidden\n");
    await writeFile(join(root, "consumers/tests/disk_ignored.py"), "import impl.common.hidden\n");
    const files = [{ path: "consumers/probe.py", source: "import impl.common.hidden\n", language: "python" },
      { path: "second/probe.py", source: "import impl.common.hidden\n", language: "python" },
      { path: "consumers/tests/ignored.py", source: "import impl.common.hidden\n", language: "python" },
      { path: "unselected/probe.py", source: "import impl.common.hidden\n", language: "python" }];
    const options = { repositoryRoot: root, packDirectory: "packs/data-layer", ruleIds: ["data-layer-matching-architecture"], mode,
      snapshot: { format: "rerail.repository-snapshot/v1", revision: 0, files }, applicationRoots: ["consumers", "second"] };
    const result = await runDataLayerAudit(options);
    expect(result.failures).toEqual([]);
    expect(result.violations.map(row => [row.code, row.path, row.span!.startLine + 1]).sort()).toEqual([
      ["MA010", "consumers/discovered.py", 1], ["MA010", "consumers/probe.py", 1], ["MA010", "second/probe.py", 1],
    ]);
    await expect(runDataLayerAudit({ ...options, applicationRoots: ["absent"] })).rejects.toThrow("Matching application root does not exist: absent");
    // The original glob scans the root itself as well as nested Python paths.
    await writeFile(join(root, "snapshot.json"), JSON.stringify(options.snapshot));
    let output = "";
    expect(await runDataLayerCommand(["--repo", root, "--pack", "packs/data-layer", "--snapshot", join(root, "snapshot.json"),
      "--rules", options.ruleIds[0]!, "--application-root", "consumers", "--application-root", "second", "--json", "--check"],
      { write(chunk: string) { output += chunk; return true; } })).toBe(1);
    expect(JSON.parse(output).violations.map((row: { path: string }) => row.path).sort()).toEqual(["consumers/discovered.py", "consumers/probe.py", "second/probe.py"]);
    const contracts = JSON.parse(await readFile("fixtures/host/data-layer-port/batch-10-dod-contracts.json", "utf8"));
    expect(contracts.matchingRootSources.map((row: { id: string }) => row.id)).toEqual(["utf8-bom", "bare-cr", "crlf"]);
    for (const record of contracts.matchingRootSources) {
      await writeFile(join(root, record.path), record.source);
      const result = await runDataLayerAudit({ ...options, snapshot: { ...options.snapshot, files: [] } });
      expect(result.failures).toEqual([]);
      expect(result.violations.filter(row => row.path === record.path).map(row => ({ code: row.code, path: row.path,
        line: row.span!.startLine + 1, message: row.message })), record.id).toEqual(record.expected);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});
