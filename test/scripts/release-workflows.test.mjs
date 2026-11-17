// The release workflows' guards that no script can hold, because they are conditions in the YAML: what may publish,
// and what may cancel a release action. A frozen release's candidate is pushed to a branch and built there, so "a
// push" alone no longer means "publish" (docs/release.md, "Freezing a release").

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";
import { CANDIDATE_BRANCH, RELEASE_BRANCH, releaseTrainTag } from "../../scripts/release/candidate/candidate.mjs";

const WORKFLOWS = join(dirname(fileURLToPath(import.meta.url)), "..", "..", ".github", "workflows");
const read = (name) => readFileSync(join(WORKFLOWS, name), "utf8");
const workflow = (name) => parse(read(name));
/** A workflow's text without its comments, where conditions are written. */
const conditions = (name) => read(name).split("\n").filter((line) => !line.trim().startsWith("#")).join("\n");

const PUBLISHES = "github.event_name == 'push' && github.ref_type == 'tag'";

describe("release-packs.yml", () => {
  const packs = workflow("release-packs.yml");

  it("starts on a release tag and on the candidate's branch, never on a push to main", () => {
    expect(packs.on.push.tags).toEqual(["release-*"]);
    expect(releaseTrainTag("a".repeat(40))).toMatch(/^release-/u);
    expect(packs.on.push.branches).toEqual([CANDIDATE_BRANCH]);
  });

  it("tags and publishes only on a release tag's push", () => {
    for (const job of ["tag", "publish", "first-publish"]) expect(packs.jobs[job].if, job).toContain(PUBLISHES);
    // No condition treats a push alone as a release: the candidate branch's push is one too.
    const pushes = conditions("release-packs.yml").match(/github\.event_name == 'push'[^\n]{0,30}/gu) ?? [];
    expect(pushes.length).toBeGreaterThan(0);
    for (const found of pushes) expect(found).toContain(PUBLISHES);
  });

  it("holds a release tag to a merged Release pull request's commit before anything is planned, tagged or published", () => {
    const steps = packs.jobs.plan.steps;
    const check = steps.findIndex((step) => String(step.run ?? "").includes("release-train-check.mjs"));
    expect(steps[check].if).toBe(PUBLISHES);
    expect(check).toBeLessThan(steps.findIndex((step) => step.id === "plan"));
    for (const job of ["tag", "publish"]) expect([packs.jobs[job].needs].flat(), job).toContain("plan");
  });

  it("gives the publish environment and write access only to jobs that follow that condition or a promotion by hand", () => {
    const gated = (name, seen = new Set()) => {
      const job = packs.jobs[name];
      if (seen.has(name)) return false;
      seen.add(name);
      const condition = String(job.if ?? "");
      if (condition.includes(PUBLISHES) || condition.includes("github.event_name == 'workflow_dispatch'")) return true;
      return [job.needs ?? []].flat().some((needed) => gated(needed, seen));
    };
    const privileged = Object.entries(packs.jobs).filter(([, job]) => job.environment !== undefined || job.permissions?.contents === "write");
    expect(privileged.map(([name]) => name).sort()).toEqual(["github-releases", "promote", "publish", "tag"]);
    for (const [name] of privileged) expect(gated(name), name).toBe(true);
  });
});

describe("release.yml", () => {
  it("publishes only from a version tag or a retry from main, so a candidate's build never does", () => {
    const { jobs } = workflow("release.yml");
    expect(jobs.publish.if).toBe("(github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')) || (github.event_name == 'workflow_dispatch' && inputs.publish != '' && github.ref == 'refs/heads/main')");
    expect(jobs["put-back"].if).toContain("github.ref == 'refs/heads/main'");
  });
});

describe("release-pr.yml", () => {
  const release = workflow("release-pr.yml");

  it("gives each action run by hand its own group, so none is cancelled or dropped, and lets only the latest push to main run", () => {
    expect(release.concurrency.group).toBe("release-pr-${{ github.event_name == 'push' && 'push' || github.run_id }}");
    expect(release.concurrency["cancel-in-progress"]).toBe("${{ github.event_name == 'push' }}");
  });

  it("offers the four actions, and names the branches the scripts use", () => {
    expect(release.on.workflow_dispatch.inputs.action.options).toEqual(["freeze", "cherry-pick", "unfreeze", "set-version"]);
    expect(release.env).toEqual({ RELEASE_BRANCH, CANDIDATE_BRANCH });
  });

  it("passes what was typed into the form through the environment, never into a command", () => {
    for (const job of Object.values(release.jobs)) {
      for (const step of job.steps) expect(String(step.run ?? ""), step.name).not.toMatch(/\$\{\{\s*inputs\./u);
    }
  });
});

describe("ci.yml", () => {
  it("runs the Release pull request's merge guard inside the one required check", () => {
    const { jobs } = workflow("ci.yml");
    expect(jobs["release-candidate"].if).toBe("needs.changes.outputs.release-pr == 'true'");
    expect(jobs["ci-result"].needs).toContain("release-candidate");
    expect(jobs.changes.outputs["release-pr"]).toContain(`'${RELEASE_BRANCH}'`);
  });
});
