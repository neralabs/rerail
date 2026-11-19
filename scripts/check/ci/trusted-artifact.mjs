#!/usr/bin/env node
// The newest artifact of a name that a build may start from (.github/actions/cargo-target, rust-compile-cache,
// engine-artifact; package.yml's release builds through rust-compile-cache): one a run of this repository uploaded
//   - on `ref` (main, unless a manual run names another branch) from an event only that branch's own history makes: a
//     push, the nightly schedule, a manual dispatch; or
//   - on `own`, the running pull request's branch when it is not `ref`, from any event of this repository: its earlier
//     pushes.
// A pull request runs the workflow its base and head merge into, so a pull request whose head branch is named `main`
// (a fork's, or this repository's against a base whose workflow uploads what it likes) reports `main` as its head
// branch, but its artifacts are never main's: its event is `pull_request`. A run whose head repository is not this one
// (a fork) is never read.
//
//   node scripts/check/ci/trusted-artifact.mjs <name> <ref> [<own>]
//     `<artifact id> <run id>` on stdout, or nothing when no artifact qualifies or the lookup fails (a cold build).
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The events of a run that only a branch's own history makes, never a pull request. */
export const BRANCH_EVENTS = new Set(["push", "schedule", "workflow_dispatch"]);

/** Whether `run` (the API's workflow run) ran this repository's code, not a fork's. */
const ownRepository = (run) => run.head_repository?.id === run.repository?.id;

/** Whether `run` is `ref`'s own: a push, schedule or dispatch on it. */
const onRef = (run, ref) => run.head_branch === ref && BRANCH_EVENTS.has(run.event);

/** Whether `run` is one of the running pull request's own earlier runs (`own` is its branch, `ref` the base's). */
const onOwn = (run, { ref, own }) => own !== undefined && own !== "" && own !== ref && run.head_branch === own;

/** Whether a build on `branches` (`{ ref, own }`) may start from what `run` uploaded. */
export function trusted(run, branches) {
  return ownRepository(run) && (onRef(run, branches.ref) || onOwn(run, branches));
}

/** Whether an artifact's summary of its run can name a trusted run at all, before the run is read. */
function candidate(artifact, { ref, own }) {
  const run = artifact.workflow_run ?? {};
  return !artifact.expired && run.head_repository_id === run.repository_id && [ref, own].includes(run.head_branch);
}

/**
 * The newest artifact named `name` that a trusted run uploaded: `{ artifact, run }` ids, or undefined. `api(path)`
 * returns the parsed JSON of a GET of the repository's REST path (`actions/artifacts?…`, `actions/runs/<id>`).
 */
export function findArtifact(api, name, branches) {
  const { artifacts = [] } = api(`actions/artifacts?name=${encodeURIComponent(name)}&per_page=50`);
  for (const artifact of artifacts) {
    if (!candidate(artifact, branches)) continue;
    const run = api(`actions/runs/${artifact.workflow_run.id}`);
    if (trusted(run, branches)) return { artifact: artifact.id, run: run.id };
  }
  return undefined;
}

/** A GET of `path` under this repository through gh (GH_TOKEN, GITHUB_REPOSITORY). */
function gh(path) {
  const out = spawnSync("gh", ["api", `repos/${process.env.GITHUB_REPOSITORY}/${path}`], { encoding: "utf8", maxBuffer: 1 << 26 });
  if (out.status !== 0) throw new Error(`gh api ${path}: ${out.stderr || out.error}`);
  return JSON.parse(out.stdout);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [name, ref, own] = process.argv.slice(2);
  try {
    const found = findArtifact(gh, name, { ref, own });
    if (found !== undefined) console.log(`${found.artifact} ${found.run}`);
  } catch (error) {
    console.error(`trusted-artifact: ${error.message}`);
  }
}
