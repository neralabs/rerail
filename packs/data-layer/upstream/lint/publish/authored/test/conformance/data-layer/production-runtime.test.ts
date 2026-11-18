import { mkdtemp, mkdir, readFile, writeFile, rm, utimes } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { expect, it } from "vitest";
import { runDataLayerAudit, runDataLayerCommand } from "../../../src/cli/index.js";

const model = `from types import SimpleNamespace
from pathlib import Path
PlanGenerationId = PlanNamespaceId = str
class SemanticPlanBuilder:
    ordinal = 0
    def __init__(self, **kwargs): self.next = 0
    def allocate_ref(self, **kwargs): return 'source'
    def add_step(self, *args): pass
    def add_return(self, *args): pass
    def freeze(self):
        mode = Path('mode.txt').read_text()
        if mode == 'error': raise RuntimeError('application probe failed')
        type(self).ordinal += 1
        value = str(type(self).ordinal) if mode == 'unstable' else 'same'
        return SimpleNamespace(fingerprint=value, as_fragment=lambda: 'fragment')
    def add_fragment(self, *args):
        if False: RefRemap; rewrite_exact_plan_refs_payload
        self.next += 1
        value = 'collision' if Path('mode.txt').read_text() == 'collision' else str(self.next)
        return SimpleNamespace(ref_map={'source': value})
`;
for (const mode of ["snapshot", "workspace"] as const) it(`collects fresh application probes through the production ${mode} runner`, async () => {
  const root = await mkdtemp(join(tmpdir(), "rerail-production-probe-"));
  const module = join(root, "l2070_batching/semantic_plan.py");
  try {
    await mkdir(join(root, "l2070_batching"), { recursive: true }); await writeFile(module, model);
    const step = join(root, "impl/common/l2070_server_plan/steps"); await mkdir(step, { recursive: true });
    await writeFile(join(step, "collect_results.py"), "class StaticValueAfterDepsStep:\n    def __init__(self, *args): pass\n");
    const options = { repositoryRoot: root, packDirectory: resolve("packs/data-layer"), ruleIds: ["data-layer-plan-composition-architecture"], mode,
      snapshot: { format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "l2070_batching/semantic_plan.py", language: "python", source: await readFile(module, "utf8") }], facts: [] } };
    for (const [setting, expected] of [["good", []], ["unstable", ["SPC-012"]], ["collision", ["SPC-005"]], ["good", []]] as const) {
      await writeFile(join(root, "mode.txt"), setting);
      const result = await runDataLayerAudit(options);
      expect(result.failures).toEqual([]); expect(result.violations.map(r => r.code)).toEqual(expected);
    }
    await expect(runDataLayerAudit({ ...options, snapshot: { ...options.snapshot, files: options.snapshot.files.map(file => ({ ...file, source: file.source + "\n" })) } })).rejects.toThrow("snapshot does not match");
    await utimes(module, 1_700_000_000, 1_700_000_000);
    execFileSync("python3.13", ["-c", "import py_compile,sys;py_compile.compile(sys.argv[1],doraise=True,invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)", module]);
    const changed = model.replace("mode == 'unstable'", "mode != 'unstable'");
    expect(changed.length).toBe(model.length);
    await writeFile(module, changed); await utimes(module, 1_700_000_000, 1_700_000_000);
    const edited = await runDataLayerAudit({ ...options, snapshot: { ...options.snapshot, files: options.snapshot.files.map(file => ({ ...file, source: changed })) } });
    expect(edited.failures).toEqual([]); expect(edited.violations.map(r => r.code)).toEqual(["SPC-012"]);
    await writeFile(module, model); await utimes(module, 1_700_000_000, 1_700_000_000);
    const snapshot = join(root, "snapshot.json"); await writeFile(snapshot, JSON.stringify(options.snapshot));
    let output = "";
    const command = await runDataLayerCommand(["--repo", root, "--pack", options.packDirectory, "--snapshot", snapshot, "--rules", options.ruleIds[0]!, "--json", "--check"], { write(chunk: string) { output += chunk; return true; } });
    expect(command).toBe(0); expect(JSON.parse(output)).toMatchObject({ violations: [], failures: [] });
    await writeFile(join(root, "mode.txt"), "collision"); output = "";
    expect(await runDataLayerCommand(["--repo", root, "--pack", options.packDirectory, "--snapshot", snapshot, "--rules", options.ruleIds[0]!, "--json", "--check"], { write(chunk: string) { output += chunk; return true; } })).toBe(1);
    expect(JSON.parse(output).violations.map((r: { code: string }) => r.code)).toEqual(["SPC-005"]);
    await writeFile(join(root, "mode.txt"), "error");
    await expect(runDataLayerAudit(options)).rejects.toThrow("application probe failed");
  } finally { await rm(root, { recursive: true, force: true }); }
}, 30_000);
