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

  it("refuses, in the publish job itself, a version that would move its channel back, since a re-run reuses metadata's outputs", () => {
    const steps = jobs.publish.steps;
    const refusal = runs(steps, "publishRefusal(");
    expect(refusal).toBeGreaterThan(-1);
    expect(refusal).toBeLessThanOrEqual(runs(steps, "npm publish"));
    const publish = steps[runs(steps, "npm publish")].run;
    expect(publish.indexOf("publishRefusal(")).toBeLessThan(publish.indexOf("npm publish"));
  });

  it("raises next only when latest moves, never on a publish", () => {
    expect(runs(jobs.publish.steps, "raise-next.mjs")).toBe(-1);
    expect(runs(jobs.promote.steps, "raise-next.mjs")).toBeGreaterThan(-1);
  });

  it("ends a release run with the promotion of what it published, waiting for a person to approve it", () => {
    const { promote, metadata } = jobs;
    expect(promote.environment).toBe("release");
    expect(promote.needs).toEqual(["metadata", "publish"]);
    expect(metadata.outputs.promotable).toBe("${{ steps.release-channel.outputs.promotable }}");
    expect(promote.if).toContain("needs.metadata.outputs.promotable == 'true' && needs.publish.result == 'success'");
    expect(promote.if).toContain("github.event_name == 'workflow_dispatch' && inputs.latest");
  });

  it("refuses, once approved, to move latest back past a newer release, though a put-back by hand may", () => {
    const steps = jobs.promote.steps;
    const guard = runs(steps, "superseded");
    expect(guard).toBeGreaterThan(-1);
    expect(guard).toBeLessThan(runs(steps, "npm dist-tag add"));
    expect(steps[guard].if).toBe("${{ !inputs.latest }}");
  });

  it("makes the promoted version's GitHub Release the latest in a job of its own, so the publish's Release never waits on the approval", () => {
    expect(jobs["github-release"].needs).not.toContain("promote");
    const promoted = jobs["promoted-release"];
    expect(promoted.needs).toContain("promote");
    expect(promoted.permissions.contents).toBe("write");
    expect(promoted.steps.find((step) => String(step.run ?? "").includes("github-release.mjs")).env.RELEASED).toContain(":latest");
  });

  it("moves latest only to a released plain version", () => {
    const steps = jobs.promote.steps;
    const guard = steps.findIndex((step) => String(step.run ?? "").includes("^[0-9]+\\.[0-9]+\\.[0-9]+$") && String(step.run ?? "").includes("git/ref/tags/v${VERSION}"));
    expect(guard).toBeGreaterThan(-1);
    expect(guard).toBeLessThan(runs(steps, "npm dist-tag add"));
  });
});

describe("release-packs.yml", () => {
  const { jobs } = workflow("release-packs.yml");

  it("starts, after a release run's publish, a promotion of exactly what it published", () => {
    const request = jobs["request-promotion"];
    expect(request.needs).toBe("publish");
    expect(request.permissions).toEqual({ actions: "write" });
    expect(request.steps[0].env.RELEASED).toBe("${{ needs.publish.outputs.released }}");
    expect(request.steps[0].run).toContain('gh workflow run release-packs.yml --ref main -f released="$RELEASED"');
    expect(jobs["promote-plan"].steps.find((step) => step.id === "plan").env.RELEASED).toBe("${{ inputs.released }}");
  });

  it("gives a release run's promotion a group of its own, so waiting for approval holds back no later release or put-back", () => {
    const { concurrency } = workflow("release-packs.yml");
    expect(concurrency.group).toContain("format('release-packs-promote-{0}', github.run_id)");
    expect(concurrency.group).toContain("inputs.released != ''");
  });

  it("installs nothing the promote job runs with id-token write, so no install script reaches it", () => {
    expect(jobs.promote.steps.some((step) => step.uses === "./.github/actions/node-deps")).toBe(false);
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

describe("every release workflow", () => {
  it("reads only the outputs of the jobs each job needs, which are all GitHub gives it", () => {
    for (const name of ["release.yml", "release-packs.yml"]) {
      for (const [id, job] of Object.entries(workflow(name).jobs)) {
        const needs = [job.needs ?? []].flat();
        const read = [...JSON.stringify(job).matchAll(/needs\.([a-z0-9-]+)\./gu)].map((match) => match[1]);
        for (const job_ of new Set(read)) expect(needs, `${name} ${id} reads needs.${job_}`).toContain(job_);
      }
    }
  });
});

