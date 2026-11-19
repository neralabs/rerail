// scripts/check/ci/trusted-artifact.mjs: which run's artifact a CI or release build may start from. Main's artifacts
// come only from main's own pushes, schedules and dispatches in this repository, never from a pull request whose head
// branch is named main.
import { describe, expect, it } from "vitest";
import { findArtifact, trusted } from "../../../scripts/check/ci/trusted-artifact.mjs";

const REPOSITORY = 1;
const FORK = 2;

/** A workflow run as the REST API returns it. */
const run = (id, { branch = "main", event = "push", head = REPOSITORY } = {}) => ({
  id,
  event,
  head_branch: branch,
  repository: { id: REPOSITORY },
  head_repository: { id: head },
});

/** An artifact listing's entry for `of`, a run from `run`. */
const artifact = (id, of, expired = false) => ({
  id,
  expired,
  workflow_run: { id: of.id, head_branch: of.head_branch, repository_id: of.repository.id, head_repository_id: of.head_repository.id },
});

/** The API over `runs` and their artifacts, newest first, recording the paths it was asked for. */
function api(entries) {
  const runs = new Map(entries.map(([, of]) => [of.id, of]));
  const asked = [];
  const get = (path) => {
    asked.push(path);
    if (path.startsWith("actions/artifacts?")) return { artifacts: entries.map(([id, of, expired]) => artifact(id, of, expired)) };
    return runs.get(Number(path.slice("actions/runs/".length)));
  };
  return { get, asked };
}

describe("trusted", () => {
  const main = { ref: "main", own: "" };

  it("takes main's pushes, schedules and dispatches in this repository", () => {
    for (const event of ["push", "schedule", "workflow_dispatch"]) expect(trusted(run(1, { event }), main)).toBe(true);
  });

  it("refuses a pull request whose head branch is named main, from this repository or a fork", () => {
    expect(trusted(run(1, { event: "pull_request" }), main)).toBe(false);
    expect(trusted(run(1, { event: "pull_request_target" }), main)).toBe(false);
    expect(trusted(run(1, { event: "pull_request", head: FORK }), main)).toBe(false);
    expect(trusted(run(1, { event: "push", head: FORK }), main)).toBe(false);
  });

  it("refuses another branch's runs", () => {
    expect(trusted(run(1, { branch: "feature" }), main)).toBe(false);
  });

  it("takes the running pull request's own earlier runs, never a fork's", () => {
    const own = { ref: "main", own: "feature" };
    expect(trusted(run(1, { branch: "feature", event: "pull_request" }), own)).toBe(true);
    expect(trusted(run(1, { branch: "feature", event: "pull_request", head: FORK }), own)).toBe(false);
  });

  it("gives a run on main no more trust when its own branch is main", () => {
    expect(trusted(run(1, { event: "pull_request" }), { ref: "main", own: "main" })).toBe(false);
  });
});

describe("findArtifact", () => {
  it("skips a newer pull request's artifact for main's push, reading only the runs that could qualify", () => {
    const { get, asked } = api([
      [10, run(100, { event: "pull_request" })],
      [11, run(101, { branch: "feature" })],
      [12, run(102, { head: FORK })],
      [13, run(103), true],
      [14, run(104, { event: "schedule" })],
      [15, run(105)],
    ]);
    expect(findArtifact(get, "rust-compile-cache-Linux-X64-release-x", { ref: "main", own: "" })).toEqual({ artifact: 14, run: 104 });
    expect(asked).toEqual(["actions/artifacts?name=rust-compile-cache-Linux-X64-release-x&per_page=50", "actions/runs/100", "actions/runs/104"]);
  });

  it("finds nothing when only pull requests uploaded on main", () => {
    const { get } = api([[10, run(100, { event: "pull_request" })]]);
    expect(findArtifact(get, "engine-x", { ref: "main", own: "" })).toBeUndefined();
  });
});
