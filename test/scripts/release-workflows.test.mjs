// The release workflows' guards that live in their YAML, where no script can hold them: what a release tag publishes
// to, and what may move `latest` (docs/release.md, "Promoting").

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";

const WORKFLOWS = join(dirname(fileURLToPath(import.meta.url)), "..", "..", ".github", "workflows");
const workflow = (name) => parse(readFileSync(join(WORKFLOWS, name), "utf8"));
const runs = (steps, text) => steps.findIndex((step) => String(step.run ?? "").includes(text));

describe("release.yml", () => {
  const { jobs } = workflow("release.yml");

  it("publishes a release tag through releaseChannel, so a plain version goes to next", () => {
    const derive = jobs.metadata.steps.find((step) => step.id === "release-channel");
    expect(derive.run).toContain("releaseChannel(");
    expect(derive.run).not.toContain('?? "latest"');
  });

  it("raises next only when latest moves, never on a publish", () => {
    expect(runs(jobs.publish.steps, "raise-next.mjs")).toBe(-1);
    expect(runs(jobs["put-back"].steps, "raise-next.mjs")).toBeGreaterThan(-1);
  });

  it("moves latest only to a released plain version", () => {
    const steps = jobs["put-back"].steps;
    const guard = steps.findIndex((step) => String(step.run ?? "").includes("^[0-9]+\\.[0-9]+\\.[0-9]+$") && String(step.run ?? "").includes("git/ref/tags/v${VERSION}"));
    expect(guard).toBeGreaterThan(-1);
    expect(guard).toBeLessThan(runs(steps, "npm dist-tag add"));
  });
});
