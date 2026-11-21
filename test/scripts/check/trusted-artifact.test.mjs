// scripts/check/ci/trusted-artifact.mjs: which run's artifact a CI or release build may start from. Main's artifacts
// come only from main's own pushes, schedules and dispatches in this repository of a commit in main's history, never
// from a pull request whose head branch is named main, nor from a run on a tag named main outside main's history.
import { describe, expect, it } from "vitest";
import { findArtifact, PER_PAGE, trusted } from "../../../scripts/check/ci/trusted-artifact.mjs";

const REPOSITORY = 1;
const FORK = 2;

/** A workflow run as the REST API returns it. */
const run = (id, { branch = "main", event = "push", head = REPOSITORY, sha = "on-main" } = {}) => ({
  id,
  event,
  head_branch: branch,
  head_sha: sha,
  repository: { id: REPOSITORY },
  head_repository: { id: head },
});

/** An artifact listing's entry for `of`, a run from `run`, or one whose run the listing does not name (`null`). */
const artifact = (id, of, expired = false) => ({
  id,
  expired,
  workflow_run: of && { id: of.id, head_branch: of.head_branch, repository_id: of.repository.id, head_repository_id: of.head_repository.id },
});

/** main's history, tip last: the commits `compare/<sha>...<tip>` finds nothing on that the tip lacks. */
const MAIN = ["older", "on-main", "tip"];

/** One page of the artifact listing over `entries`, as `actions/artifacts?…&per_page=<n>&page=<p>` asks. */
function listing(entries, path) {
  const query = new URLSearchParams(path.slice(path.indexOf("?") + 1));
  const size = Number(query.get("per_page"));
  const start = (Number(query.get("page")) - 1) * size;
  return { artifacts: entries.slice(start, start + size).map(([id, of, expired]) => artifact(id, of, expired)) };
}

/**
 * The API over `runs` and their artifacts, newest first, and main's history, recording the paths it was asked for. A
 * path `fails` names throws, as gh does on an HTTP error.
 */
function api(entries, { fails = () => false } = {}) {
  const runs = new Map(entries.filter(([, of]) => of !== null).map(([, of]) => [of.id, of]));
  const asked = [];
  const get = (path) => {
    asked.push(path);
    if (fails(path)) throw new Error(`gh api ${path}: HTTP 404`);
    if (path.startsWith("actions/artifacts?")) return listing(entries, path);
    if (path === "branches/main") return { commit: { sha: MAIN.at(-1) } };
    if (path.startsWith("compare/")) return { behind_by: MAIN.includes(path.slice("compare/".length).split("...")[0]) ? 0 : 1 };
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
    expect(asked).toEqual([
      `actions/artifacts?name=rust-compile-cache-Linux-X64-release-x&per_page=${PER_PAGE}&page=1`,
      "actions/runs/100",
      "actions/runs/104",
      "branches/main",
      "compare/on-main...tip?per_page=1",
    ]);
  });

  it("finds nothing when only pull requests uploaded on main", () => {
    const { get } = api([[10, run(100, { event: "pull_request" })]]);
    expect(findArtifact(get, "engine-x", { ref: "main", own: "" })).toBeUndefined();
  });

  it("refuses a push of a tag named main: its commit is not in main's history", () => {
    const { get, asked } = api([
      [10, run(100, { sha: "tagged" })],
      [11, run(101)],
    ]);
    expect(findArtifact(get, "engine-x", { ref: "main", own: "" })).toEqual({ artifact: 11, run: 101 });
    expect(asked).toContain("compare/tagged...tip?per_page=1");
  });

  it("takes main's tip without a comparison, and reads the pull request's own runs without one", () => {
    const tip = api([[10, run(100, { sha: "tip" })]]);
    expect(findArtifact(tip.get, "engine-x", { ref: "main", own: "" })).toEqual({ artifact: 10, run: 100 });
    expect(tip.asked.some((path) => path.startsWith("compare/"))).toBe(false);
    const own = api([[10, run(100, { branch: "feature", event: "pull_request", sha: "off-main" })]]);
    expect(findArtifact(own.get, "engine-x", { ref: "main", own: "feature" })).toEqual({ artifact: 10, run: 100 });
    expect(own.asked).toEqual([`actions/artifacts?name=engine-x&per_page=${PER_PAGE}&page=1`, "actions/runs/100"]);
  });

  it("skips an artifact whose run the listing does not name, as the CLI's two-argument form asks", () => {
    const { get } = api([
      [10, null],
      [11, run(101)],
    ]);
    expect(findArtifact(get, "engine-x", { ref: "main", own: undefined })).toEqual({ artifact: 11, run: 101 });
  });

  it("counts out a commit whose comparison fails and still finds an older artifact of main's", () => {
    const { get } = api(
      [
        [10, run(100, { sha: "orphan" })],
        [11, run(101)],
      ],
      { fails: (path) => path.startsWith("compare/orphan...") },
    );
    expect(findArtifact(get, "engine-x", { ref: "main", own: "" })).toEqual({ artifact: 11, run: 101 });
  });

  it("reads the pull request's own runs when the base's tip can't be read", () => {
    const { get, asked } = api(
      [
        [10, run(100)],
        [11, run(101)],
        [12, run(102, { branch: "feature", event: "pull_request", sha: "off-main" })],
      ],
      { fails: (path) => path === "branches/main" },
    );
    expect(findArtifact(get, "engine-x", { ref: "main", own: "feature" })).toEqual({ artifact: 12, run: 102 });
    expect(asked.filter((path) => path === "branches/main")).toHaveLength(1);
  });

  it("reads on past a full page of pull requests' artifacts to main's, and stops at a short page", () => {
    const pullRequests = Array.from({ length: PER_PAGE }, (_, k) => [k, run(1000 + k, { event: "pull_request" })]);
    const { get, asked } = api([...pullRequests, [PER_PAGE, run(2000)]]);
    expect(findArtifact(get, "engine-x", { ref: "main", own: "" })).toEqual({ artifact: PER_PAGE, run: 2000 });
    expect(asked.filter((path) => path.startsWith("actions/artifacts?"))).toHaveLength(2);
    const none = api(pullRequests.slice(1));
    expect(findArtifact(none.get, "engine-x", { ref: "main", own: "" })).toBeUndefined();
    expect(none.asked.filter((path) => path.startsWith("actions/artifacts?"))).toHaveLength(1);
  });
});
