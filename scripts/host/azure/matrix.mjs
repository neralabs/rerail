#!/usr/bin/env node
// Installs a Rerail build on throwaway Azure VMs, one per environment in environments.json (Linux distributions
// on x64 and arm64, Windows 11 on x64 and arm64), and checks it there the way a user meets it: `npx
// @neralabs/rerail setup`, the managed runtime, PATH in new shells, the coding-agent hooks, daemon restart, and
// removal. See docs/environment-matrix.md.
//
//   node scripts/host/azure/matrix.mjs run --tarball <neralabs-rerail-*.tgz> [--env <id,...>] [--keep]
//   node scripts/host/azure/matrix.mjs run --npm <version or dist-tag> [--env <id,...>]
//   node scripts/host/azure/matrix.mjs list [--env <id,...>]     JSON array of ids, for a CI matrix
//   node scripts/host/azure/matrix.mjs sweep [--older-than-hours <n>]
//
// Everything goes through the `az` CLI, signed in (`az login` locally, OIDC in CI) to the subscription holding the
// resource group that scripts/host/azure/bootstrap.sh made. No VM accepts inbound connections: scripts go in through
// Azure Run Command, the package and results move through short-lived blob SAS URLs, and each VM reaches npmjs,
// nodejs.org and GitHub through its own public IP. Every resource a run creates carries its run id and is deleted
// when the environment finishes; `sweep` removes what a cancelled run left.
import { execFile } from "node:child_process";
import { randomBytes } from "node:crypto";
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs, promisify } from "node:util";

const run = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "../../..");
const MATRIX = JSON.parse(readFileSync(join(HERE, "environments.json"), "utf8"));
for (const environment of MATRIX.environments) {
  const problem = !/^[a-z0-9.-]+$/.test(environment.id) ? "an id of lowercase letters, digits, dots and dashes"
    : !["linux", "windows"].includes(environment.os) ? 'os "linux" or "windows"'
    : !environment.image || !environment.size ? "an image and a size"
    : environment.expect === "setup-fails" ? (!environment.failsWith || !environment.because ? "failsWith and because" : null)
    : environment.expect !== "pass" ? 'expect "pass" or "setup-fails"' : null;
  if (problem) throw new Error(`environments.json: ${environment.id ?? "an environment"} needs ${problem}`);
}
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

function selectEnvironments(only) {
  if (only === undefined || only === "" || only === "all") return MATRIX.environments;
  const ids = only.split(",").map((id) => id.trim()).filter(Boolean);
  const unknown = ids.filter((id) => !MATRIX.environments.some((environment) => environment.id === id));
  if (unknown.length > 0) {
    throw new Error(`Unknown environment ${unknown.join(", ")}; known: ${MATRIX.environments.map((e) => e.id).join(", ")}`);
  }
  return MATRIX.environments.filter((environment) => ids.includes(environment.id));
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

// Inline settings the guest scripts read: shell assignments for bash, a hashtable for Windows PowerShell.
function guestScript(environment, settings) {
  if (environment.os === "linux") {
    const quote = (value) => `'${String(value).replaceAll("'", "'\\''")}'`;
    const header = Object.entries(settings).map(([key, value]) => `RERAIL_ENV_${key.replace(/[A-Z]/g, (c, i) => (i ? "_" : "") + c).toUpperCase()}=${quote(value)}`);
    const body = readFileSync(join(HERE, "guest.sh"), "utf8").replace(/^#!.*\n/, "");
    return `#!/usr/bin/env bash\n${header.join("\n")}\n${body}`;
  }
  const quote = (value) => `'${String(value).replaceAll("'", "''")}'`;
  const header = `$RerailEnv = @{ ${Object.entries(settings).map(([key, value]) => `${key} = ${quote(value)}`).join("; ")} }`;
  return `${header}\n${readFileSync(join(HERE, "guest.ps1"), "utf8")}`;
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
  const vm = `rerail-${environment.id.replaceAll(".", "")}-${runId}`.slice(0, 64);
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
      ResultUrl: await store.sas(container, `${environment.id}/results.json`, "cw"),
      LogUrl: await store.sas(container, `${environment.id}/guest.log`, "cw"),
      ...(environment.os === "windows" ? { Password: password } : {}),
    };
    view = await runCommand(environment, vm, guestScript(environment, settings), started);
    log(environment, `Run Command finished: ${view.executionState}, exit ${view.exitCode}`);
    // Windows provisions, restarts and signs the user in; the checks then run in that session and end by uploading
    // results.json, which is what to wait for.
    if (environment.os === "windows" && view.executionState === "Succeeded") {
      log(environment, "waiting for the signed-in user's checks");
      while (!(await store.exists(container, `${environment.id}/results.json`))) {
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
      if (await store.download(container, `${environment.id}/results.json`, resultsFile)) {
        results = JSON.parse(readFileSync(resultsFile, "utf8").replace(/^\uFEFF/, ""));
      }
      await store.download(container, `${environment.id}/guest.log`, join(outDir, `${environment.id}.log`));
    } catch (error) {
      log(environment, `could not fetch results: ${error.message}`);
    }
    if (keep) {
      log(environment, `--keep: leaving ${vm} in ${RESOURCE_GROUP} (admin ${ADMIN_USER}); delete it with sweep`);
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

function report(outcomes, packageLabel) {
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

async function commandRun(options) {
  if (Boolean(options.tarball) === Boolean(options.npm)) throw new Error("Pass exactly one of --tarball <file> or --npm <version or dist-tag>");
  const environments = selectEnvironments(options.env);
  const runId = options["run-id"] ?? `${new Date().toISOString().slice(0, 10).replaceAll("-", "")}${randomBytes(3).toString("hex")}`;
  if (!/^[a-z0-9-]{1,32}$/.test(runId)) throw new Error("--run-id takes lowercase letters, digits and dashes, at most 32");
  const outDir = resolve(options.out ?? join(REPO_ROOT, ".rerail", "env-matrix", runId));
  mkdirSync(outDir, { recursive: true });
  const store = await storage();
  const container = `run-${runId}`;
  await store.container(container);
  let bundleSas;
  let packageLabel = `@neralabs/rerail@${options.npm} from npmjs`;
  if (options.tarball) {
    const tarball = resolve(options.tarball);
    if (!existsSync(tarball)) throw new Error(`No tarball at ${tarball}`);
    packageLabel = basename(tarball);
    // Matrix jobs of one CI run share the container; the first to get here uploads.
    if (!(await store.exists(container, "bundle.tar"))) {
      const { bundle, contents } = await makeBundle(tarball);
      console.log(`bundle: ${contents.join(", ")}`);
      await store.upload(container, "bundle.tar", bundle);
    }
    bundleSas = await store.sas(container, "bundle.tar", "r");
  }
  console.log(`run ${runId}: ${environments.map((e) => e.id).join(", ")} in ${RESOURCE_GROUP}; results in ${outDir}`);
  const outcomes = await Promise.all(environments.map((environment) =>
    runEnvironment(environment, { runId, container, store, bundleSas, npmSpec: options.npm, outDir, keep: options.keep })));
  const markdown = report(outcomes, packageLabel);
  writeFileSync(join(outDir, "summary.md"), markdown);
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, markdown);
  for (const { environment, verdict } of outcomes) {
    console.log(`${environment.id}: ${verdict.outcome}${verdict.reason ? ` (${verdict.reason})` : ""}`);
  }
  console.log(`summary: ${join(outDir, "summary.md")}`);
  if (!outcomes.every(({ verdict }) => OK_OUTCOMES.has(verdict.outcome))) process.exitCode = 1;
}

async function commandSweep(options) {
  const hours = Number(options["older-than-hours"] ?? 3);
  const cutoff = Date.now() - hours * 60 * 60_000;
  const tagged = await az(["resource", "list", "-g", RESOURCE_GROUP, "--query", `[?tags.purpose=='${PURPOSE}' && tags.run != null].{run: tags.run, created: tags.created}`]);
  const stale = [...new Set(tagged.filter((r) => !r.created || Date.parse(r.created) < cutoff).map((r) => r.run))];
  for (const runId of stale) {
    console.log(`deleting what run ${runId} left`);
    await deleteRun(runId);
  }
  console.log(stale.length === 0 ? `nothing older than ${hours} h` : `swept ${stale.length} run(s)`);
}

const { values: options, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    env: { type: "string" },
    tarball: { type: "string" },
    npm: { type: "string" },
    "run-id": { type: "string" },
    out: { type: "string" },
    keep: { type: "boolean", default: false },
    "older-than-hours": { type: "string" },
  },
});

try {
  switch (positionals[0]) {
    case "run": await commandRun(options); break;
    case "list": console.log(JSON.stringify(selectEnvironments(options.env).map((e) => e.id))); break;
    case "sweep": await commandSweep(options); break;
    default: throw new Error("Usage: matrix.mjs run (--tarball <file> | --npm <spec>) [--env <ids>] [--keep] | list [--env <ids>] | sweep [--older-than-hours <n>]");
  }
} catch (error) {
  console.error(`matrix: ${error.message}`);
  process.exitCode = 1;
}
