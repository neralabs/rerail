import { mkdtemp, readdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, it } from "vitest";
import { createExperimentStore } from "../src/experiments.js";
import type { PlaygroundExperiment } from "../src/protocol.js";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});
async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-experiments-test-"));
  directories.push(root);
  return root;
}
function experiment(): PlaygroundExperiment {
  const selection = { ruleId: "review-rule", scenarioId: "positive", mode: "trigger" as const, allowCommands: false };
  const scenario = {
    id: "positive",
    title: "Review a fixture",
    expectation: "match" as const,
    explanation: "Predicted finding",
    ruleDigest: "original-rule",
    ruleKind: "conversation" as const,
    prompt: "Review fixture.ts",
    fixtures: [{ path: "fixture.ts", content: "const value = 1;" }],
    steps: [],
  };
  return {
    ruleId: selection.ruleId,
    selection,
    scenarios: [scenario],
    lastRun: {
      selection,
      scenario,
      rule: {
        id: selection.ruleId,
        kind: "file",
        definition: "{}",
        digest: "edited-rule",
        updatedAt: "2",
        source: "library",
      },
      outcome: { observed: "cannot-exercise", detail: "Rule kind changed; scenario needs revision" },
      completedAt: 3,
      trace: [],
    },
  };
}

it("restores an incompatible DB-edit run without rewriting scenario provenance or losing its result", async () => {
  const root = await temporaryRoot();
  const saved = experiment();
  await createExperimentStore(root).save(saved);
  expect(await createExperimentStore(root).load(saved.ruleId)).toEqual(saved);
});

it("starts fresh instead of failing every restore when a saved experiment predates the protocol", async () => {
  const root = await temporaryRoot();
  const store = createExperimentStore(root);
  const saved = experiment();
  await store.save(saved);
  const [file] = await readdir(join(root, ".rerail", "playground"));
  await writeFile(join(root, ".rerail", "playground", file!), JSON.stringify({
    ...saved,
    scenarios: saved.scenarios.map((scenario) => ({ ...scenario, ruleKind: "cosine" })),
  }));
  expect(await store.load(saved.ruleId)).toBeUndefined();
  await store.save(saved);
  expect(await store.load(saved.ruleId)).toEqual(saved);
});

it("rejects traversal in an edited scenario without replacing the last saved experiment", async () => {
  const root = await temporaryRoot();
  const store = createExperimentStore(root);
  const saved = experiment();
  await store.save(saved);
  const unsafe = structuredClone(saved);
  unsafe.scenarios[0]!.fixtures[0]!.path = "../outside.ts";
  await expect(store.save(unsafe)).rejects.toThrow("sandbox-relative");
  expect(await createExperimentStore(root).load(saved.ruleId)).toEqual(saved);
});

it("refuses symlinked experiment storage without touching its external target", async () => {
  const root = await temporaryRoot();
  const outside = await temporaryRoot();
  await writeFile(join(outside, "sentinel"), "unchanged");
  await symlink(outside, join(root, ".rerail"));
  await expect(createExperimentStore(root).save(experiment())).rejects.toThrow("real directory");
  expect(await readFile(join(outside, "sentinel"), "utf8")).toBe("unchanged");
});
