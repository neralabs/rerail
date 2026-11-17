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

describe("release-packs.yml", () => {
  const { jobs } = workflow("release-packs.yml");

  it("installs the dependencies the promote plan reads core ranges with, before npm is given the read token", () => {
    const steps = jobs["promote-plan"].steps;
    const deps = steps.findIndex((step) => step.uses === "./.github/actions/node-deps");
    expect(deps).toBeGreaterThan(-1);
    expect(deps).toBeLessThan(runs(steps, ".npmrc"));
  });

  it("makes each promoted pack's GitHub Release a full release", () => {
    expect(jobs.promote.outputs.promoted).toBe("${{ steps.promote.outputs.promoted }}");
    const releases = jobs["promoted-releases"];
    expect(releases.needs).toBe("promote");
    expect(releases.permissions.contents).toBe("write");
    const step = releases.steps.find((candidate) => String(candidate.run ?? "").includes("github-release.mjs"));
    expect(step.env.RELEASED).toBe("${{ needs.promote.outputs.promoted }}");
  });
});
