#!/usr/bin/env node
// Installs a Rerail build in each environment of environments.json (Linux distributions on x64 and arm64 and
// Windows 11 on x64 and arm64, on throwaway Azure VMs; macOS on GitHub-hosted runners, since Azure has no Macs) and
// checks it there the way a user meets it: `npx @neralabs/rerail setup`, the managed runtime, PATH in new shells, the
// coding-agent hooks, daemon restart, and removal. See docs/environment-matrix.md.
//
//   node scripts/host/azure/matrix.mjs run --tarball <neralabs-rerail-*.tgz> [--env <id,...>] [--keep]
//   node scripts/host/azure/matrix.mjs run --npm <version or dist-tag> [--env <id,...>]
//   node scripts/host/azure/matrix.mjs stage --tarball <file> --run-id <id>     bundle once for a CI run's jobs, which
//   node scripts/host/azure/matrix.mjs run --staged --run-id <id> [--env <id,...>]      then each run --staged
//   node scripts/host/azure/matrix.mjs local --tarball <file> [--env <id>]     this GitHub runner (sudo): a macOS id, or as itself
//   node scripts/host/azure/matrix.mjs list --on <azure|github> [--env <id,...>]     JSON, for a CI matrix
//   node scripts/host/azure/matrix.mjs affected --base <commit> --head <commit>     true when the matrix should run
//   node scripts/host/azure/matrix.mjs sweep [--run-id <id> | --older-than-hours <n>]
//
// The Azure environments go through the `az` CLI, signed in (`az login` locally, OIDC in CI) to the subscription
// holding the resource group that scripts/host/azure/bootstrap.sh made. No VM accepts inbound connections: scripts go
// in through Azure Run Command, the package and results move through short-lived blob SAS URLs, and each VM reaches
// npmjs, nodejs.org and GitHub through its own public IP. Every resource a run creates carries its run id and is
// deleted when the environment finishes; `sweep` removes what a cancelled run left. A macOS environment runs the
// same guest on the machine `local` runs on, a fresh GitHub-hosted runner in CI, and needs no Azure.
import { execFile, spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir, userInfo } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs, promisify } from "node:util";
import { matrixChanges } from "../../release/core-paths.mjs";

const run = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "../../..");
const MATRIX = JSON.parse(readFileSync(join(HERE, "environments.json"), "utf8"));
for (const environment of MATRIX.environments) {
  const problem = !/^[a-z0-9.-]+$/.test(environment.id) ? "an id of lowercase letters, digits, dots and dashes"
    : !["linux", "windows", "macos"].includes(environment.os) ? 'os "linux", "windows" or "macos"'
    : environment.os === "macos" ? (!environment.runner ? "a GitHub-hosted runner label" : null)
    : !environment.image || !environment.size ? "an image and a size"
    : null;
  const expectation = environment.expect === "setup-fails" ? (!environment.failsWith || !environment.because ? "failsWith and because" : null)
    : environment.expect !== "pass" ? 'expect "pass" or "setup-fails"' : null;
  if (problem ?? expectation) throw new Error(`environments.json: ${environment.id ?? "an environment"} needs ${problem ?? expectation}`);
}
// Where an environment runs: an Azure VM, or a GitHub-hosted macOS runner.
const hostOf = (environment) => (environment.os === "macos" ? "github" : "azure");
const PURPOSE = "rerail-env-matrix";
const RESOURCE_GROUP = process.env.RERAIL_AZURE_RESOURCE_GROUP ?? "rerail-envtest-rg";
const VNET = process.env.RERAIL_AZURE_VNET ?? "rerail-env-vnet";
const SUBNET = process.env.RERAIL_AZURE_SUBNET ?? "vms";
const ADMIN_USER = "rerail";
// Generous: a Windows VM takes ~2 min to create, ~5 to install Node and Git, and the checks ~5 more.
const ENVIRONMENT_DEADLINE_MS = 60 * 60_000;
const TERMINAL_STATES = new Set(["Succeeded", "Failed", "TimedOut", "Canceled"]);

async function az(args, { json = true } = {}) {
  try {
    const { stdout } = await run("az", [...args, ...(json ? ["--output", "json"] : ["--output", "none"])], {
      maxBuffer: 64 * 1024 * 1024,
    });
    return json && stdout.trim() !== "" ? JSON.parse(stdout) : undefined;
  } catch (error) {
    throw new Error(`az ${args.slice(0, 3).join(" ")} failed: ${(error.stderr || error.message).trim()}`);
  }
}

// The environments `only` names (all of them when it names none) that run on `host`. `list` splits a mixed list
// between the two hosts; `run` and `local` refuse one that leaves them nothing, so a macOS id never runs nothing.
function selectEnvironments(only, host) {
  const onHost = MATRIX.environments.filter((environment) => hostOf(environment) === host);
  if (only === undefined || only === "" || only === "all") return onHost;
  const ids = only.split(",").map((id) => id.trim()).filter(Boolean);
  const unknown = ids.filter((id) => !MATRIX.environments.some((environment) => environment.id === id));
  if (unknown.length > 0) {
    throw new Error(`Unknown environment ${unknown.join(", ")}; known: ${MATRIX.environments.map((e) => e.id).join(", ")}`);
  }
  return onHost.filter((environment) => ids.includes(environment.id));
}

// ---- Storage: the package goes in and results come out through blob SAS URLs -------------------------------------

async function storage() {
  const name = process.env.RERAIL_AZURE_STORAGE_ACCOUNT
    ?? (await az(["storage", "account", "list", "-g", RESOURCE_GROUP, "--query", `[?tags.purpose=='${PURPOSE}'].name | [0]`]));
  if (!name) throw new Error(`No storage account tagged purpose=${PURPOSE} in ${RESOURCE_GROUP}; run scripts/host/azure/bootstrap.sh`);
  const key = await az(["storage", "account", "keys", "list", "-g", RESOURCE_GROUP, "-n", name, "--query", "[0].value"]);
  const auth = ["--account-name", name, "--account-key", key];
  const expiry = new Date(Date.now() + 3 * 60 * 60_000).toISOString().slice(0, 16) + "Z";
  return {
    async container(container) {
      await az(["storage", "container", "create", ...auth, "-n", container], { json: false });
    },
    containers(prefix) {
      return az(["storage", "container", "list", ...auth, "--prefix", prefix, "--query", "[].{name: name, modified: properties.lastModified}"]);
    },
    // When anything was last written to the container: its newest blob, or the container itself when it has none.
    // Writing a blob leaves the container's own lastModified as it was.
    async lastWritten({ name, modified }) {
      const blobs = await az(["storage", "blob", "list", ...auth, "-c", name, "--num-results", "*", "--query", "[].properties.lastModified"]);
      return Math.max(Date.parse(modified), ...blobs.map((time) => Date.parse(time)));
    },
    async deleteContainer(container) {
      await az(["storage", "container", "delete", ...auth, "-n", container], { json: false });
    },
    async upload(container, blob, file) {
      await az(["storage", "blob", "upload", ...auth, "-c", container, "-n", blob, "-f", file, "--overwrite", "--no-progress"], { json: false });
    },
    async exists(container, blob) {
      return (await az(["storage", "blob", "exists", ...auth, "-c", container, "-n", blob])).exists;
    },
    async download(container, blob, file) {
      if (!(await this.exists(container, blob))) return false;
      await az(["storage", "blob", "download", ...auth, "-c", container, "-n", blob, "-f", file, "--no-progress"], { json: false });
      return true;
    },
    sas(container, blob, permissions) {
      return az(["storage", "blob", "generate-sas", ...auth, "-c", container, "-n", blob, "--permissions", permissions,
        "--expiry", expiry, "--https-only", "--full-uri"]);
    },
  };
}

// The package under test and every rule pack from this checkout, in one tar the VM unpacks: the guest serves them
// from a local registry, as npmjs will.
async function makeBundle(tarball) {
  const directory = mkdtempSync(join(tmpdir(), "rerail-env-bundle-"));
  writeFileSync(join(directory, basename(tarball)), readFileSync(tarball));
  const packs = join(REPO_ROOT, "packs");
  for (const pack of readdirSync(packs).filter((name) => existsSync(join(packs, name, "package.json")))) {
    await run("npm", ["pack", join(packs, pack), "--pack-destination", directory, "--ignore-scripts", "--silent"]);
  }
  const bundle = join(tmpdir(), `rerail-env-bundle-${Date.now()}.tar`);
  // COPYFILE_DISABLE: macOS tar would otherwise add a `._<name>` AppleDouble file beside every tarball.
  await run("tar", ["-cf", bundle, "-C", directory, "."], { env: { ...process.env, COPYFILE_DISABLE: "1" } });
  return { bundle, contents: readdirSync(directory) };
}

// ---- One environment ---------------------------------------------------------------------------------------------

const log = (environment, message) => console.log(`[${environment.id}] ${message}`);

// The script the guest runs: the settings it reads (shell assignments for bash, a hashtable for Windows PowerShell),
// then the guest itself; on Windows with scripts/host/windows-host.ps1, which it shares with the release gate, ahead
// of it. Linux and macOS share guest.sh.
function guestScript(environment, settings) {
  if (environment.os !== "windows") {
    const quote = (value) => `'${String(value).replaceAll("'", "'\\''")}'`;
    const header = Object.entries(settings).map(([key, value]) => `RERAIL_ENV_${key.replace(/[A-Z]/g, (c, i) => (i ? "_" : "") + c).toUpperCase()}=${quote(value)}`);
    const body = readFileSync(join(HERE, "guest.sh"), "utf8").replace(/^#!.*\n/, "");
    return `#!/usr/bin/env bash\n${header.join("\n")}\n${body}`;
  }
  const quote = (value) => `'${String(value).replaceAll("'", "''")}'`;
  const header = `$RerailEnv = @{ ${Object.entries(settings).map(([key, value]) => `${key} = ${quote(value)}`).join("; ")} }`;
  const shared = readFileSync(join(HERE, "..", "windows-host.ps1"), "utf8");
  return `${header}\n${shared}\n${readFileSync(join(HERE, "guest.ps1"), "utf8")}`;
}

async function createVm(environment, { runId, vm, password }) {
  const computerName = `rr${randomBytes(4).toString("hex")}`; // Windows allows 15 characters.
  const subnet = await az(["network", "vnet", "subnet", "show", "-g", RESOURCE_GROUP, "--vnet-name", VNET, "-n", SUBNET, "--query", "id"]);
  await az([
    "vm", "create", "-g", RESOURCE_GROUP, "-n", vm, "--computer-name", computerName,
    "--image", environment.image, "--size", environment.size,
    "--admin-username", ADMIN_USER, "--admin-password", password, "--authentication-type", "password",
    // The subnet's network security group admits nothing from outside; the public IP only carries outbound traffic.
    "--subnet", subnet, "--nsg", "", "--public-ip-address", `${vm}-ip`, "--public-ip-sku", "Standard",
    "--storage-sku", "StandardSSD_LRS", "--os-disk-delete-option", "Delete", "--nic-delete-option", "Delete",
    ...(environment.license ? ["--license-type", environment.license] : []),
    "--tags", `purpose=${PURPOSE}`, `run=${runId}`, `env=${environment.id}`, `created=${new Date().toISOString()}`,
  ]);
}

async function deleteRun(runId, environmentId) {
  const filter = `[?tags.run=='${runId}'${environmentId ? ` && tags.env=='${environmentId}'` : ""}]`;
  const vms = await az(["vm", "list", "-g", RESOURCE_GROUP, "--query", `${filter}.id`]);
  if (vms.length > 0) await az(["vm", "delete", "--ids", ...vms, "--yes", "--force-deletion", "true"], { json: false });
  // The VM takes its NIC and disk with it; its public IP stays and goes here, with anything else of the run's.
  const rest = await az(["resource", "list", "-g", RESOURCE_GROUP, "--query", `${filter}.id`]);
  const ips = await az(["network", "public-ip", "list", "-g", RESOURCE_GROUP, "--query", `${filter}.id`]);
  const leftovers = [...new Set([...rest, ...ips])];
  if (leftovers.length > 0) await az(["resource", "delete", "--ids", ...leftovers], { json: false });
}

async function runCommand(environment, vm, script, started) {
  const file = join(mkdtempSync(join(tmpdir(), "rerail-env-script-")), environment.os === "linux" ? "guest.sh" : "guest.ps1");
  writeFileSync(file, script);
  // Both guests provision as root/SYSTEM and switch to the ordinary user themselves (Linux: a login shell; Windows:
  // a console sign-in), so Run Command runs as its default account.
  await az(["vm", "run-command", "create", "-g", RESOURCE_GROUP, "--vm-name", vm, "--run-command-name", "rerail-env",
    "--script", `@${file}`, "--timeout-in-seconds", String(Math.floor(ENVIRONMENT_DEADLINE_MS / 1000) - 600),
    "--async-execution", "true"], { json: false });
  const deadline = started + ENVIRONMENT_DEADLINE_MS;
  for (;;) {
    const view = await az(["vm", "run-command", "show", "-g", RESOURCE_GROUP, "--vm-name", vm, "--run-command-name", "rerail-env",
      "--instance-view", "--query", "instanceView"]);
    if (view && TERMINAL_STATES.has(view.executionState)) return view;
    if (Date.now() > deadline) return { executionState: "TimedOut", exitCode: null, output: "", error: "the driver gave up waiting" };
    await new Promise((done) => setTimeout(done, 20_000));
  }
}

// Judges the guest's results against what the environment is expected to do. An expected set-up failure counts only
// when set-up failed the expected way (`failsWith` in its output or the log): failing for another reason, a full
// disk say, is a failure of the run.
function judge(environment, results, log) {
  if (results === undefined) return { outcome: "error", reason: "the VM uploaded no results" };
  if (!results.provisioned) return { outcome: "error", reason: "provisioning the VM failed" };
  const setup = results.steps.find((step) => step.name === "setup");
  const failed = results.steps.find((step) => step.ok === false);
  if (environment.expect === "setup-fails") {
    if (setup?.ok === false) {
      if (setup.detail.includes(environment.failsWith) || log.includes(environment.failsWith)) {
        return { outcome: "expected-fail", reason: environment.because };
      }
      return { outcome: "fail", reason: `set-up failed, but not with "${environment.failsWith}"`, step: setup };
    }
    return {
      outcome: "unexpected-pass",
      reason: `set-up now succeeds here (${environment.because} no longer holds?): update environments.json`,
    };
  }
  if (failed !== undefined) return { outcome: "fail", reason: `${failed.name} failed`, step: failed };
  return { outcome: "pass", reason: "" };
}

async function runEnvironment(environment, { runId, container, store, bundleSas, npmSpec, outDir, keep }) {
  // One CI run keeps its run id across re-run attempts (they share its staged bundle), so each attempt at an
  // environment gets its own VM and result blobs: a re-run never reads the results an earlier attempt left.
  const attempt = randomBytes(2).toString("hex");
  const vm = `rerail-${environment.id.replaceAll(".", "")}-${runId}-${attempt}`.slice(0, 64);
  const blobs = `${environment.id}/${attempt}`;
  const password = `Rr1!${randomBytes(16).toString("hex")}`;
  if (process.env.GITHUB_ACTIONS === "true") console.log(`::add-mask::${password}`);
  const started = Date.now();
  let results;
  let view;
  try {
    log(environment, `creating ${vm} (${environment.image}, ${environment.size})`);
    await createVm(environment, { runId, vm, password });
    log(environment, `VM ready after ${Math.round((Date.now() - started) / 1000)} s; provisioning and checking`);
    const settings = {
      Id: environment.id,
      User: ADMIN_USER,
      Node: environment.node ?? MATRIX.node,
      BundleUrl: bundleSas ?? "",
      NpmSpec: npmSpec ?? "",
      NpmToken: npmSpec ? (process.env.RERAIL_ENV_NPM_TOKEN ?? "") : "",
      ResultUrl: await store.sas(container, `${blobs}/results.json`, "cw"),
      LogUrl: await store.sas(container, `${blobs}/guest.log`, "cw"),
      ...(environment.os === "windows" ? { Password: password } : {}),
    };
    view = await runCommand(environment, vm, guestScript(environment, settings), started);
    log(environment, `Run Command finished: ${view.executionState}, exit ${view.exitCode}`);
    // Windows provisions, restarts and signs the user in; the checks then run in that session and end by uploading
    // results.json, which is what to wait for.
    if (environment.os === "windows" && view.executionState === "Succeeded") {
      log(environment, "waiting for the signed-in user's checks");
      while (!(await store.exists(container, `${blobs}/results.json`))) {
        if (Date.now() - started > ENVIRONMENT_DEADLINE_MS) {
          view = { executionState: "TimedOut", error: "the signed-in user's checks uploaded no results in time" };
          break;
        }
        await new Promise((done) => setTimeout(done, 20_000));
      }
    }
  } catch (error) {
    log(environment, `driver error: ${error.message}`);
    view = { executionState: "DriverError", error: error.message };
  } finally {
    const resultsFile = join(outDir, `${environment.id}.json`);
    try {
      if (await store.download(container, `${blobs}/results.json`, resultsFile)) {
        results = JSON.parse(readFileSync(resultsFile, "utf8").replace(/^\uFEFF/, ""));
      }
      await store.download(container, `${blobs}/guest.log`, join(outDir, `${environment.id}.log`));
    } catch (error) {
      log(environment, `could not fetch results: ${error.message}`);
    }
    if (keep) {
      log(environment, `--keep: leaving ${vm} in ${RESOURCE_GROUP} (admin ${ADMIN_USER}); delete it with sweep --run-id ${runId}`);
    } else {
      try {
        await deleteRun(runId, environment.id);
        log(environment, "deleted its VM, disk, NIC and public IP");
      } catch (error) {
        log(environment, `cleanup failed (sweep will retry): ${error.message}`);
      }
    }
  }
  const logFile = join(outDir, `${environment.id}.log`);
  const verdict = judge(environment, results, existsSync(logFile) ? readFileSync(logFile, "utf8") : "");
  if (verdict.outcome === "error" && view?.error) verdict.reason += `: ${String(view.error).slice(-800)}`;
  return { environment, results, verdict, minutes: Math.round((Date.now() - started) / 6000) / 10 };
}

// ---- Report ------------------------------------------------------------------------------------------------------

const OK_OUTCOMES = new Set(["pass", "expected-fail"]);
const MARK = { pass: "✅ pass", "expected-fail": "☑️ fails as expected", fail: "❌ fail", "unexpected-pass": "⚠️ now passes", error: "💥 error" };

function summary(outcomes, packageLabel) {
  const facts = (results) => {
    const f = results?.facts ?? {};
    return [f.os, f.build, f.glibc, f.node && `Node ${f.node.replace(/^v/, "")}`].filter(Boolean).join(" · ");
  };
  const lines = [
    `### Rerail environment matrix: ${packageLabel}`,
    "",
    "| Environment | Expected | Outcome | Steps | Details | Minutes |",
    "| -- | -- | -- | -- | -- | -- |",
    ...outcomes.map(({ environment, results, verdict, minutes }) => {
      const steps = (results?.steps ?? []).map((step) => `${step.ok === true ? "✓" : step.ok === false ? "✗" : "–"} ${step.name}`).join("<br>");
      const expected = environment.expect === "pass" ? "pass" : `set-up fails: ${environment.because}`;
      return `| ${environment.id} | ${expected} | ${MARK[verdict.outcome]} | ${steps || "—"} | ${facts(results)}${verdict.reason ? `<br>${verdict.reason}` : ""} | ${minutes} |`;
    }),
  ];
  for (const { environment, results, verdict } of outcomes) {
    const step = verdict.step ?? (verdict.outcome === "expected-fail" || verdict.outcome === "unexpected-pass"
      ? results?.steps.find((s) => s.name === "setup")
      : undefined);
    if (!step?.detail) continue;
    lines.push("", `<details><summary>${environment.id}: ${step.name}</summary>`, "", "```", step.detail.trim(), "```", "", "</details>");
  }
  return lines.join("\n") + "\n";
}

// ---- Commands ----------------------------------------------------------------------------------------------------

const newRunId = () => `${new Date().toISOString().slice(0, 10).replaceAll("-", "")}${randomBytes(3).toString("hex")}`;

function checkRunId(runId) {
  if (!/^[a-z0-9-]{1,32}$/.test(runId)) throw new Error("--run-id takes lowercase letters, digits and dashes, at most 32");
  return runId;
}

// Bundles a tarball (and every rule pack in packs/) into the run's container, where each environment's VM fetches
// it. Done once per run: CI's environment jobs all run --staged against it.
async function stage(store, container, tarball) {
  if (!existsSync(tarball)) throw new Error(`No tarball at ${tarball}`);
  const { bundle, contents } = await makeBundle(tarball);
  console.log(`bundle: ${contents.join(", ")}`);
  await store.upload(container, "bundle.tar", bundle);
  const label = join(mkdtempSync(join(tmpdir(), "rerail-env-label-")), "package.txt");
  writeFileSync(label, basename(tarball));
  await store.upload(container, "package.txt", label);
  return basename(tarball);
}

async function commandStage(options) {
  if (!options.tarball || !options["run-id"]) throw new Error("stage needs --tarball <file> and --run-id <id>");
  const store = await storage();
  const container = `run-${checkRunId(options["run-id"])}`;
  await store.container(container);
  console.log(`staged ${await stage(store, container, resolve(options.tarball))} in ${container}`);
}

async function commandRun(options) {
  if ([options.tarball, options.npm, options.staged].filter(Boolean).length !== 1) {
    throw new Error("Pass exactly one of --tarball <file>, --npm <version or dist-tag>, or --staged");
  }
  if (options.staged && !options["run-id"]) throw new Error("--staged runs against what `stage` put in --run-id's container");
  const environments = selectEnvironments(options.env, "azure");
  if (environments.length === 0) throw new Error(`${options.env} runs on a GitHub-hosted macOS runner, not Azure: see \`local\``);
  const runId = checkRunId(options["run-id"] ?? newRunId());
  const outDir = resolve(options.out ?? join(REPO_ROOT, ".rerail", "env-matrix", runId));
  mkdirSync(outDir, { recursive: true });
  const store = await storage();
  const container = `run-${runId}`;
  let bundleSas;
  let packageLabel = `@neralabs/rerail@${options.npm} from npmjs`;
  if (options.staged) {
    const label = join(outDir, "package.txt");
    if (!(await store.download(container, "package.txt", label))) {
      throw new Error(`Nothing is staged in ${container}: run stage first. In CI, a re-run that starts over three hours after the run last wrote anything finds its bundle swept; re-run all jobs`);
    }
    packageLabel = readFileSync(label, "utf8").trim();
    bundleSas = await store.sas(container, "bundle.tar", "r");
  } else {
    await store.container(container);
    if (options.tarball) {
      packageLabel = await stage(store, container, resolve(options.tarball));
      bundleSas = await store.sas(container, "bundle.tar", "r");
    }
  }
  console.log(`run ${runId}: ${environments.map((e) => e.id).join(", ")} in ${RESOURCE_GROUP}; results in ${outDir}`);
  const outcomes = await Promise.all(environments.map((environment) =>
    runEnvironment(environment, { runId, container, store, bundleSas, npmSpec: options.npm, outDir, keep: options.keep })));
  report(outcomes, packageLabel, outDir);
}

// Writes the summary (and the job summary in CI), prints each outcome, and fails the command unless all are as
// expected.
function report(outcomes, packageLabel, outDir) {
  const markdown = summary(outcomes, packageLabel);
  writeFileSync(join(outDir, "summary.md"), markdown);
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, markdown);
  for (const { environment, verdict } of outcomes) {
    console.log(`${environment.id}: ${verdict.outcome}${verdict.reason ? ` (${verdict.reason})` : ""}`);
  }
  console.log(`summary: ${join(outDir, "summary.md")}`);
  if (!outcomes.every(({ verdict }) => OK_OUTCOMES.has(verdict.outcome))) process.exitCode = 1;
}

// One environment on the machine this runs on: the guest provisions as root (sudo) and checks as this account, as it
// does on an Azure VM. `--env` names a macOS environment of the matrix; without it, this runner is the environment
// (`<os>-<arch>`, expected to pass), as the release workflow's Linux and macOS gates run it. It installs Node
// system-wide, writes to this account's home and on a Mac makes its shell zsh, so it runs only on a GitHub-hosted
// Linux or macOS runner, which is thrown away afterwards.
async function commandLocal(options) {
  if (!options.tarball) throw new Error("local needs --tarball <file> [--env <macOS environment id>]");
  const os = { darwin: "macos", linux: "linux" }[process.platform];
  let environment = { id: `${os}-${process.arch}`, os, expect: "pass" };
  if (options.env) {
    const environments = selectEnvironments(options.env, "github");
    if (environments.length !== 1) throw new Error("local runs exactly one macOS environment: --env <id>");
    [environment] = environments;
  }
  if (os === undefined || environment.os !== os || process.env.RUNNER_ENVIRONMENT !== "github-hosted") {
    throw new Error(`${environment.id ?? "local"} runs only on a GitHub-hosted ${environment.os ?? "Linux or macOS"} runner: it changes the account and installs Node system-wide`);
  }
  const tarball = resolve(options.tarball);
  if (!existsSync(tarball)) throw new Error(`No tarball at ${tarball}`);
  const outDir = resolve(options.out ?? join(REPO_ROOT, ".rerail", "env-matrix", environment.id));
  mkdirSync(outDir, { recursive: true });
  const { bundle, contents } = await makeBundle(tarball);
  console.log(`bundle: ${contents.join(", ")}`);
  const user = userInfo().username;
  const settings = { Id: environment.id, User: user, Node: environment.node ?? MATRIX.node, BundleUrl: `file://${bundle}` };
  const file = join(mkdtempSync(join(tmpdir(), "rerail-env-script-")), "guest.sh");
  writeFileSync(file, guestScript(environment, settings));
  const started = Date.now();
  log(environment, `provisioning as root, then checking as ${user}`);
  const status = await new Promise((done) => spawn("sudo", ["bash", file], { stdio: "inherit" }).on("close", done));
  log(environment, `guest exited ${status} after ${Math.round((Date.now() - started) / 1000)} s`);
  // Where the guest leaves results.json and its log when it has no blob URLs to upload them to.
  const work = "/opt/rerail-env";
  let results;
  if (existsSync(join(work, "results.json"))) {
    const text = readFileSync(join(work, "results.json"), "utf8");
    writeFileSync(join(outDir, `${environment.id}.json`), text);
    results = JSON.parse(text);
  }
  const logFile = ["all.log", "provision.log"].map((name) => join(work, name)).find(existsSync);
  const guestLog = logFile ? readFileSync(logFile, "utf8") : "";
  writeFileSync(join(outDir, `${environment.id}.log`), guestLog);
  const verdict = judge(environment, results, guestLog);
  report([{ environment, results, verdict, minutes: Math.round((Date.now() - started) / 6000) / 10 }], basename(tarball), outDir);
}

// Whether a push from `base` to `head` should run the matrix (core-paths.mjs `matrixChanges`). Prints the reason to
// stderr and `true` or `false` to stdout.
async function commandAffected(options) {
  if (!options.base || !options.head) throw new Error("affected needs --base <commit> and --head <commit>");
  if (/^0+$/.test(options.base)) {
    console.error("no previous head to compare with");
    console.log("true");
    return;
  }
  const git = async (args) => (await run("git", args, { cwd: REPO_ROOT, maxBuffer: 64 * 1024 * 1024 })).stdout;
  const changed = (await git(["diff", "--name-only", "--no-renames", options.base, options.head])).split("\n").filter(Boolean);
  const lock = {};
  for (const ref of ["release", "head"]) lock[ref] = changed.includes("package-lock.json") ? await git(["show", `${ref === "release" ? options.base : options.head}:package-lock.json`]) : "";
  const { core, matrix } = matrixChanges(changed, (ref) => lock[ref]);
  const reasons = [...core.slice(0, 5), ...matrix.slice(0, 5)];
  console.error(reasons.length === 0
    ? `nothing core ships or the matrix runs changed (${changed.length} file(s) changed)`
    : `changed: ${reasons.join(", ")}${core.length + matrix.length > reasons.length ? ", …" : ""}`);
  console.log(String(reasons.length > 0));
}

// Deletes what runs left: with --run-id, everything of that one run, whatever its age (a --keep run, say); otherwise
// every run's resources older than --older-than-hours (default 3, longer than a run takes, so a run still going keeps
// its VMs), and every storage container nothing was written to for that long (each attempt writes its results there,
// so a run's bundle lasts three hours past its last attempt, for re-running failed jobs).
async function commandSweep(options) {
  const store = await storage();
  if (options["run-id"]) {
    const runId = checkRunId(options["run-id"]);
    await deleteRun(runId);
    if ((await store.containers(`run-${runId}`)).some((c) => c.name === `run-${runId}`)) await store.deleteContainer(`run-${runId}`);
    console.log(`swept run ${runId}`);
    return;
  }
  const hours = Number(options["older-than-hours"] ?? 3);
  const cutoff = Date.now() - hours * 60 * 60_000;
  const tagged = await az(["resource", "list", "-g", RESOURCE_GROUP, "--query", `[?tags.purpose=='${PURPOSE}' && tags.run != null].{run: tags.run, created: tags.created}`]);
  const stale = [...new Set(tagged.filter((r) => !r.created || Date.parse(r.created) < cutoff).map((r) => r.run))];
  for (const runId of stale) {
    console.log(`deleting what run ${runId} left`);
    await deleteRun(runId);
  }
  // A run's container holds its bundle and results; its environments share it, so it goes here, once nothing has
  // written to it for the cutoff, rather than when one environment finishes. The storage account's lifecycle rule
  // expires blobs, not containers.
  const containers = [];
  for (const container of await store.containers("run-")) {
    if ((await store.lastWritten(container)) < cutoff) containers.push(container);
  }
  for (const { name } of containers) await store.deleteContainer(name);
  console.log(`swept ${stale.length} run(s) and ${containers.length} storage container(s) idle for ${hours} h`);
}

const { values: options, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    env: { type: "string" },
    tarball: { type: "string" },
    npm: { type: "string" },
    staged: { type: "boolean", default: false },
    "run-id": { type: "string" },
    out: { type: "string" },
    keep: { type: "boolean", default: false },
    "older-than-hours": { type: "string" },
    base: { type: "string" },
    head: { type: "string" },
    on: { type: "string" },
  },
});

// A CI matrix: the Azure environments' ids, or the macOS environments' ids with the runner each needs.
function commandList(options) {
  if (options.on !== "azure" && options.on !== "github") throw new Error("list needs --on azure or --on github");
  const environments = selectEnvironments(options.env, options.on);
  console.log(JSON.stringify(options.on === "azure" ? environments.map((e) => e.id) : environments.map((e) => ({ id: e.id, runner: e.runner }))));
}

try {
  switch (positionals[0]) {
    case "run": await commandRun(options); break;
    case "stage": await commandStage(options); break;
    case "local": await commandLocal(options); break;
    case "list": commandList(options); break;
    case "affected": await commandAffected(options); break;
    case "sweep": await commandSweep(options); break;
    default: throw new Error("Usage: matrix.mjs run (--tarball <file> | --npm <spec> | --staged --run-id <id>) [--env <ids>] [--keep] | stage --tarball <file> --run-id <id> | local --tarball <file> [--env <id>] | list --on <azure|github> [--env <ids>] | affected --base <commit> --head <commit> | sweep [--run-id <id> | --older-than-hours <n>]");
  }
} catch (error) {
  console.error(`matrix: ${error.message}`);
  process.exitCode = 1;
}
