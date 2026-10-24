import { spawnSync } from "node:child_process";
import { expect, it } from "vitest";

const required = process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1";
const engine = new URL("../../../src/engine/index.ts", import.meta.url).href;
const caps = new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;
const policy = new URL("../../../packs/data-layer/candidates/rules/data-layer-serverplan-state-tripwires.json", import.meta.url).href;

it.skipIf(!required)("keeps admission and physical producers identical for permutations of the same data-layer plans", () => {
  const program = `
    import {readFileSync} from 'node:fs';
    import {loadNativeBinding} from ${JSON.stringify(engine)};
    const native=loadNativeBinding();
    const compiled=native.CodeRuleDslRuntime.compile(readFileSync(new URL(${JSON.stringify(policy)}),'utf8'),'json',readFileSync(new URL(${JSON.stringify(caps)}),'utf8'));
    if(typeof compiled.candidateFrontierAudit!=='function')throw new Error('The addon must expose the real compiler audit');
    console.log(compiled.candidateFrontierAudit());
  `;
  const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    encoding: "utf8", timeout: 45_000, maxBuffer: 16 * 1024 * 1024,
  });
  expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
  const report = JSON.parse(child.stdout.trim());
  expect(report.languages.length).toBeGreaterThan(0);
  for (const language of report.languages) {
    expect(language.variants.map((v: { order: string }) => v.order)).toEqual(["forward", "reverse", "rotated", "interleaved"]);
    const first = language.variants[0];
    for (const field of ["stageSha256", "producerSha256", "parametersSha256"])
      expect(first[field], field).toMatch(/^[0-9a-f]{64}$/);
    expect(first.plans).toBeGreaterThan(1);
    expect(first.admissions.some((a: [number, number, boolean]) => a[2])).toBe(true);
    for (const variant of language.variants.slice(1)) {
      expect(variant.admissions, `${variant.order}: cache warmth must not change admission or the per-plan bound`).toEqual(first.admissions);
      expect(variant.stageSha256, `${variant.order}: producer placement must not follow allocation or caller order`).toBe(first.stageSha256);
      expect(variant.producerSha256).toBe(first.producerSha256);
      expect(variant.parametersSha256).toBe(first.parametersSha256);
      expect(variant.resultQueries).toBe(first.resultQueries);
      expect(variant.peakLiveRelations).toBe(first.peakLiveRelations);
    }
  }
  expect(report.deterministic).toBe(true);
}, 50_000);
