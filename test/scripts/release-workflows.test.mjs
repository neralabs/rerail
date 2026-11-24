// The release workflows' guards that live in their YAML, where no script can hold them: what a release tag publishes
// to, the order its seven packages are packed and published in (docs/release.md, "Engine packages"), and what may move
// `latest` (docs/release.md, "Promoting").

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";

const WORKFLOWS = join(dirname(fileURLToPath(import.meta.url)), "..", "..", ".github", "workflows");
const workflow = (name) => parse(readFileSync(join(WORKFLOWS, name), "utf8"));
const runs = (steps, text) => steps.findIndex((step) => String(step.run ?? "").includes(text));
/** The folder package.yml packs the seven tarballs into and uploads. */
const PACKED = "${{ runner.temp }}/rerail-package";

describe("package.yml", () => {
  const { jobs, on } = workflow("package.yml");
  const steps = jobs.package.steps;

  it("packs all six engine packages, then core against them, and checks the tarball before anything installs it", () => {
    const engines = runs(steps, "scripts/release/engine/packages.mjs pack");
    const core = runs(steps, "npm pack");
    const check = runs(steps, "scripts/release/shrinkwrap-pack.mjs check");
    const smoke = runs(steps, "npm install --prefix");
    expect(engines).toBeGreaterThan(steps.findIndex((step) => step.name === "Download native engines"));
    expect(steps[engines].run).toBe('node scripts/release/engine/packages.mjs pack --all --out "$RUNNER_TEMP/rerail-package"');
    expect(core).toBeGreaterThan(engines);
    expect(check).toBeGreaterThan(core);
    expect(smoke).toBeGreaterThan(check);
    // One folder throughout: the pins come from its manifest, and the check hashes its tarballs.
    for (const index of [core, check, smoke]) expect(steps[index].env.RERAIL_ENGINE_PACKAGES, steps[index].name).toBe(PACKED);
    expect(steps[core].run).toContain('npm pack --json --pack-destination "$RERAIL_ENGINE_PACKAGES"');
    expect(steps[check].run).toBe('node scripts/release/shrinkwrap-pack.mjs check "$RERAIL_ENGINE_PACKAGES/$PACKAGE_TARBALL"');
  });

  it("installs core together with the runner's engine package, since core's tarball alone installs with no engine", () => {
    const smoke = steps[runs(steps, "npm install --prefix")].run;
    expect(smoke).toContain('npm install --prefix "$smoke_directory" "$RERAIL_ENGINE_PACKAGES/$PACKAGE_TARBALL" "$RERAIL_ENGINE_PACKAGES/$engine_tarball"');
    expect(smoke).toContain("enginePackageFor().name");
    expect(smoke.indexOf("npm install --prefix")).toBeLessThan(smoke.indexOf("nativeHealth().ok"));
    expect(smoke).toContain('compgen -G "$installed/native/*.node"');
  });

  it("uploads the seven tarballs and the manifest as one artifact, and still names core's tarball for the workflows that call it", () => {
    const upload = steps.at(-1);
    expect(upload.with).toMatchObject({ name: "rerail-package", path: PACKED, "if-no-files-found": "error" });
    expect(jobs.package.outputs.tarball).toBe("${{ steps.packed-package.outputs.tarball }}");
    expect(on.workflow_call.outputs.tarball.value).toBe("${{ jobs.package.outputs.tarball }}");
  });
});

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
    // Before the first of the seven publishes, which is an engine package's.
    expect(refusal).toBeLessThan(runs(steps, "engine/release.mjs publish"));
    expect(refusal).toBeLessThan(runs(steps, "npm publish"));
  });

  it("publishes the six engine packages and reads them back before core, so core is never on npm ahead of an engine it pins", () => {
    const steps = jobs.publish.steps;
    const engines = runs(steps, "engine/release.mjs publish");
    const core = runs(steps, "npm publish");
    expect(engines).toBeGreaterThan(-1);
    expect(core).toBeGreaterThan(engines);
    // The one `npm publish` the workflow spells is core's; the script publishes the engine packages and reads them back.
    expect(steps.filter((step) => String(step.run ?? "").includes("npm publish"))).toHaveLength(1);
    expect(steps[engines].run).toBe("node scripts/release/engine/release.mjs publish release");
    expect(steps[core].env.PACKAGE_TARBALL).toBe("release/${{ needs.package.outputs.tarball }}");
    expect(steps[engines].env).toEqual({ RELEASE_CHANNEL: "${{ needs.metadata.outputs.channel }}", RELEASE_VERSION: "${{ needs.metadata.outputs.version }}" });
    // Core's publish runs only when the step before it succeeded: nothing lets it run past a failed read-back.
    for (const step of steps.slice(engines)) {
      expect(step.if, step.name).toBeUndefined();
      expect(step["continue-on-error"], step.name).toBeUndefined();
    }
    expect(jobs.publish["continue-on-error"]).toBeUndefined();
  });

  it("gives the publish job time for seven uploads and the read-back", () => {
    expect(jobs.publish["timeout-minutes"]).toBeGreaterThanOrEqual(30);
  });

  it("checks the tarball against the engine packages of the artifact it came in, before anything is published", () => {
    const { shrinkwrap, publish } = jobs;
    const download = shrinkwrap.steps.find((step) => String(step.uses ?? "").startsWith("actions/download-artifact@"));
    const check = shrinkwrap.steps[runs(shrinkwrap.steps, "shrinkwrap-pack.mjs check")];
    expect(download.with).toEqual({ name: "rerail-package", path: "release" });
    expect(check.env.RERAIL_ENGINE_PACKAGES).toBe("release");
    expect(publish.needs).toContain("shrinkwrap");
  });

  it("reads back the dist-tag of core and of each engine package published with it", () => {
    const check = jobs["dist-tags"].steps[runs(jobs["dist-tags"].steps, "dist-tags-check.mjs")];
    expect(check.run).toContain('node scripts/release/engine/release.mjs specs "$VERSION" "$CHANNEL"');
    expect(check.run).toContain('RELEASED="@neralabs/rerail@${VERSION}:${CHANNEL} ${engines}" node scripts/release/dist-tags-check.mjs');
    expect(check.env).toEqual({ VERSION: "${{ needs.metadata.outputs.version }}", CHANNEL: "${{ needs.metadata.outputs.channel }}" });
  });

  it("moves latest for core and then each engine package of the version, raises next for all of them and reads all of them back", () => {
    const steps = jobs.promote.steps;
    const list = steps.findIndex((step) => step.id === "packages");
    const move = runs(steps, "npm dist-tag add");
    const raise = runs(steps, "raise-next.mjs");
    const check = runs(steps, "dist-tags-check.mjs");
    const specs = "${{ steps.packages.outputs.specs }}";
    // Core first, then whatever engine packages the version has: none for one from before them, so a put-back to it
    // moves core alone.
    expect(steps[list].run).toContain('engines="$(node scripts/release/engine/release.mjs specs "$VERSION")"');
    expect(steps[list].run).toContain('echo "specs=@neralabs/rerail@${VERSION} ${engines}" >> "$GITHUB_OUTPUT"');
    expect(list).toBeGreaterThan(runs(steps, "superseded"));
    expect([move, raise, check]).toEqual([list + 1, list + 2, list + 3]);
    expect(steps[move].env.SPECS).toBe(specs);
    expect(steps[move].run).toContain('for spec in "${specs[@]}"; do');
    expect(steps[move].run).toContain('npm dist-tag add "$spec" latest');
    expect(steps[raise].env.SPECS).toBe(specs);
    expect(steps[raise].run).toContain('node scripts/release/raise-next.mjs "${specs[@]}"');
    expect(steps[check].env.RELEASED).toBe(specs);
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

  it("keys a publish retry's run by its version's tag, so its promotion waits in the tag's group, not main's, where put-backs run", () => {
    const { concurrency } = workflow("release.yml");
    expect(concurrency.group).toBe("release-${{ inputs.publish && format('refs/tags/v{0}', inputs.publish) || github.ref }}");
    expect(concurrency["cancel-in-progress"]).toBe(false);
  });

  it("writes the promoted Release from main's changelog, so a note or lead corrected on main while the release was tested is what stable users read", () => {
    const checkout = jobs["promoted-release"].steps.find((step) => String(step.uses ?? "").startsWith("actions/checkout@"));
    expect(checkout.with.ref).toBe("main");
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

  it("writes the promoted Releases from main's changelog as it is at approval, not as it was when the promotion was requested", () => {
    const checkout = jobs["promoted-releases"].steps.find((step) => String(step.uses ?? "").startsWith("actions/checkout@"));
    expect(checkout.with.ref).toBe("main");
  });
});

describe("every release workflow", () => {
  // One table names the engine packages (ENGINE_PACKAGES in scripts/release/engine/packages.mjs), and a name is
  // permanent once published: a workflow or a host script asks the scripts for the names.
  it("spells no engine package's name", () => {
    const root = join(WORKFLOWS, "..", "..");
    const scripts = ["scripts/host/azure/guest.sh", "scripts/host/azure/guest.ps1", "scripts/host/azure/matrix.mjs", "scripts/host/windows-host.ps1", "scripts/host/windows-install-smoke.ps1"];
    const workflows = ["package.yml", "release.yml", "env-matrix.yml", "release-pr.yml"].map((name) => `.github/workflows/${name}`);
    for (const path of [...workflows, ...scripts]) expect(readFileSync(join(root, path), "utf8").includes("@neralabs/rerail-engine-"), path).toBe(false);
  });

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

describe("release-pr.yml", () => {
  const { jobs } = workflow("release-pr.yml");

  it("runs the released cores in a job that holds no write token, installing them fresh, never from a cache", () => {
    const check = jobs["core-refusals"];
    expect(check.permissions).toEqual({ contents: "read" });
    expect(JSON.stringify(check)).not.toMatch(/secrets\.|app-token|actions\/cache/u);
    expect(JSON.stringify(workflow("release-pr.yml"))).not.toContain("actions/cache");
    expect(check.outputs.refusals).toBe("${{ steps.check.outputs.refusals }}");
  });

  it("hands the Release pull request only the refusals, and versions nothing when the check did not pass", () => {
    const job = jobs["pull-request"];
    expect(job.needs).toContain("core-refusals");
    expect(job.if).toContain("needs.core-refusals.result == 'success'");
    const version = job.steps.find((step) => String(step.run ?? "").includes("release-version.mjs"));
    expect(version.env.RELEASE_CORE_REFUSALS).toBe("${{ needs.core-refusals.outputs.refusals }}");
  });
});
