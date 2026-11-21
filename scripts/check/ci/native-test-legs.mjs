#!/usr/bin/env node
// The engine's Rust tests in CI: compiled once, run in legs (ci.yml's `rust-tests-build` and `rust` jobs).
//
// `build` runs `cargo test --workspace` with this script as cargo's target runner (`capture`): cargo compiles every
// test binary and runs the doc tests (rustdoc hands them to the runner, which runs them), and hands each test binary to
// the runner, which records it with the working directory and the environment cargo gave it (the package's directory,
// CARGO_MANIFEST_DIR, CARGO_PKG_*, .cargo/config.toml's [env], the library path) instead of running it. Then it lists
// every binary's tests (`--list`) whole and under each leg's filters, and fails unless each test falls in exactly one
// leg. What it writes
// (`<dir>/native-tests.tar.zst`: the binaries at their paths under target/, and the manifest) is the artifact each
// leg unpacks in its checkout, at the same path as the build's, where `run <leg>` runs each binary with the leg's
// libtest filters as `cargo test --workspace -- <filters>` would: same working directory, same environment (the leg's
// own PATH kept), libtest as the harness. Unlike cargo, a failed binary does not stop the ones after it.
//
// The legs are libtest filters over the test paths, so a test's leg follows its module: the last leg skips what the
// others select, and the build's check keeps "every test in exactly one leg" true as tests move.
//
//   node scripts/check/ci/native-test-legs.mjs build <dir>    compile, run the doc tests, check the legs, pack; the legs
//                                                          with tests to run go to $GITHUB_OUTPUT as `legs` (JSON)
//   node scripts/check/ci/native-test-legs.mjs run <leg> <dir> run one leg's tests from the unpacked artifact
//
// NATIVE_TESTS_REPORT_TIME=1 has `run` add libtest's unstable `--report-time` (under RUSTC_BOOTSTRAP=1), each test's
// time, to rebalance the legs from a run's log.
import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const SELF = fileURLToPath(import.meta.url);
/** zstd's window for the artifact, both ways (decompression needs it named too). */
const LONG = "--long=31";

/**
 * One leg: the tests whose path holds any of `filters` (every test when empty) and none of `skip`, in the test binaries
 * `binaries` names (by crate or test target: the binary's name without its hash), or in every binary but those
 * `exclude` names.
 */
const SELECTED = [
  // Nine tests hold 25 of the 108 test-minutes, two of them 8 minutes each (scratch run 44354698909, `--report-time`):
  // the slowest leg, at 7.5 minutes. The legs run 4.7 to 7.5 minutes each (scratch run 44360932319); logic and typing
  // with the SQL executor and slice in one leg ran 9.
  { id: "waves", name: "tests, waves", filters: ["code_rule_dsl::waves::"], skip: [] },
  {
    id: "workspace",
    name: "tests, workspace, anchors and transducer",
    filters: ["code_rule_dsl::workspace::", "code_rule_dsl::matcher::anchors::", "code_rule_dsl::transducer::"],
    skip: [],
  },
  { id: "logic-rules", name: "tests, logic rules and converters", filters: ["code_rule_dsl::logic::rr::", "code_rule_dsl::logic::convert::"], skip: [] },
  {
    id: "logic",
    name: "tests, logic and typing",
    filters: ["code_rule_dsl::logic::", "code_rule_dsl::typing::"],
    skip: ["code_rule_dsl::logic::rr::", "code_rule_dsl::logic::convert::"],
  },
  { id: "sql", name: "tests, SQL executor and slice", filters: ["code_rule_dsl::sql_executor::", "code_rule_dsl::slice::"], skip: [] },
  // The rr parser's corpus checks: 4.3 and 1.5 of the last leg's 10 minutes (CI run 44383070532), two tests 2.6 and 2.2.
  { id: "parsers", name: "tests, rr parser corpora", filters: [], skip: [], binaries: ["rerail_parser_tests", "rerail_rr_parser"] },
];

/** The legs, in order; the last runs every test the others do not select. */
export const LEGS = [
  ...SELECTED,
  {
    id: "rest",
    name: "tests, the rest",
    filters: [],
    skip: SELECTED.flatMap((leg) => (leg.binaries === undefined ? leg.filters : [])),
    exclude: SELECTED.flatMap((leg) => leg.binaries ?? []),
  },
];

/** A test binary's name: its file's, without cargo's hash or `.exe`. */
export function binaryName(exe) {
  return exe.split(/[/\\]/u).at(-1).replace(/\.exe$/u, "").replace(/-[0-9a-f]{16}$/u, "");
}

/** Whether `leg` runs tests of the binary `exe`. */
export function legRuns(leg, exe) {
  const name = binaryName(exe);
  return leg.binaries === undefined ? !(leg.exclude ?? []).includes(name) : leg.binaries.includes(name);
}

/** A leg's libtest arguments. */
export function legArgs(leg) {
  return [...leg.filters, ...leg.skip.flatMap((path) => ["--skip", path])];
}

/** The tests `--list --format terse` printed: `<path>: test` lines (benchmarks are not run by `cargo test`). */
export function parseList(text) {
  return text.split("\n").filter((line) => line.endsWith(": test")).map((line) => line.slice(0, -": test".length));
}

/**
 * Each problem with the legs' cover of the tests: `all` maps a binary to its tests, `byLeg` a leg's id to a map of the
 * binary to the tests the leg selects. A test no leg or two legs select is a problem, named with its binary.
 */
export function coverProblems(all, byLeg) {
  const problems = [];
  for (const [binary, tests] of all) {
    for (const test of tests) {
      const legs = [...byLeg].filter(([, selected]) => selected.get(binary)?.has(test)).map(([id]) => id);
      if (legs.length !== 1) problems.push(`${binary}: ${test} is in ${legs.length === 0 ? "no leg" : `legs ${legs.join(", ")}`}`);
    }
  }
  for (const [id, selected] of byLeg) {
    for (const [binary, tests] of selected) {
      for (const test of tests) if (!all.get(binary)?.has(test)) problems.push(`${binary}: leg ${id} selects ${test}, which the whole list lacks`);
    }
  }
  return problems;
}

/** What cargo set or changed for a test binary: `env` against the environment cargo was started with. */
export function envDelta(env, parent) {
  const delta = {};
  for (const [key, value] of Object.entries(env)) {
    if (key.startsWith("NATIVE_TESTS_") || key === "PATH") continue;
    if (parent[key] !== value) delta[key] = value;
  }
  return delta;
}

/** The counts of libtest's summary line (`test result: ok. 3 passed; 0 failed; 1 ignored; ...`), or undefined. */
export function parseResult(line) {
  const match = /test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured/u.exec(line);
  if (match === null) return undefined;
  return { passed: Number(match[1]), failed: Number(match[2]), ignored: Number(match[3]), measured: Number(match[4]) };
}

function capture() {
  const [exe, ...args] = process.argv.slice(3);
  // rustdoc hands cargo's runner the doc tests it compiled (a crate's merged doc tests, edition 2024) from a temporary
  // directory it removes after: those run here, as cargo would run them.
  const target = resolve(process.env.CARGO_TARGET_DIR ?? join(ROOT, "target"));
  if (relative(target, resolve(exe)).startsWith("..")) {
    const doc = spawnSync(exe, args, { stdio: "inherit" });
    process.exit(doc.status ?? 1);
  }
  const parent = JSON.parse(readFileSync(process.env.NATIVE_TESTS_PARENT_ENV, "utf8"));
  const record = { exe, args, cwd: process.cwd(), env: envDelta(process.env, parent), path: process.env.PATH === parent.PATH };
  appendFileSync(process.env.NATIVE_TESTS_CAPTURE, `${JSON.stringify(record)}\n`);
}

function hostTriple() {
  const run = spawnSync("rustc", ["-vV"], { cwd: ROOT, encoding: "utf8" });
  if (run.status !== 0) throw new Error(`rustc -vV failed:\n${run.stderr}`);
  return /^host: (.+)$/mu.exec(run.stdout)[1].trim();
}

function list(binary, args) {
  const run = spawnSync(binary.exe, [...args, "--list", "--format", "terse"], {
    cwd: binary.cwd, env: { ...process.env, ...binary.env }, encoding: "utf8", maxBuffer: 1 << 28,
  });
  if (run.status !== 0) throw new Error(`${binary.exe} --list failed (${run.status ?? run.signal}):\n${run.stderr}`);
  return new Set(parseList(run.stdout));
}

/**
 * The cargo arguments that pick the packages: `npm run test:native`'s `--workspace`, or the packages
 * NATIVE_TESTS_CARGO_ARGS selects (`-p <package>` each: a pull request's changed crates and the crates that depend on
 * them, ci.yml's `changes` job).
 */
function cargoSelection() {
  const selected = (process.env.NATIVE_TESTS_CARGO_ARGS ?? "").split(/\s+/u).filter(Boolean);
  return selected.length === 0 ? ["--workspace"] : selected;
}

/** Runs `cargo test` with this script as the target runner, which records each test binary; returns the record's path. */
function compileCaptured(dir) {
  mkdirSync(dir, { recursive: true });
  const captured = join(dir, "captured.jsonl");
  const parentEnv = join(dir, "parent-env.json");
  writeFileSync(captured, "");
  const runner = `CARGO_TARGET_${hostTriple().toUpperCase().replaceAll("-", "_")}_RUNNER`;
  const env = { ...process.env, [runner]: `node ${SELF} capture`, NATIVE_TESTS_CAPTURE: captured, NATIVE_TESTS_PARENT_ENV: parentEnv };
  writeFileSync(parentEnv, JSON.stringify(env));
  const started = Date.now();
  const cargo = spawnSync("cargo", ["test", "--manifest-path", "Cargo.toml", ...cargoSelection()], { cwd: ROOT, stdio: "inherit", env });
  if (cargo.status !== 0) process.exit(cargo.status ?? 1);
  console.log(`cargo test (compile, doc tests): ${((Date.now() - started) / 60000).toFixed(1)} min`);
  return captured;
}

/** The test binaries the runner recorded, each with an absolute path; throws on what `run` could not replay. */
function capturedBinaries(captured) {
  const binaries = readFileSync(captured, "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line));
  for (const binary of binaries) {
    if (binary.args.length > 0) throw new Error(`cargo passed ${binary.exe} arguments: ${binary.args.join(" ")}`);
    if (!binary.path) throw new Error(`cargo changed PATH for ${binary.exe}; run keeps the leg's own`);
    if (!isAbsolute(binary.exe)) binary.exe = resolve(binary.cwd, binary.exe);
  }
  return binaries;
}

/** The tests `leg` selects in each binary, by the binary's path. */
function legTests(leg, binaries) {
  return new Map(binaries.map((binary) => [binary.exe, legRuns(leg, binary.exe) ? list(binary, legArgs(leg)) : new Set()]));
}

/** The number of tests in a map of binary to tests. */
function testCount(tests) {
  return [...tests.values()].reduce((sum, names) => sum + names.size, 0);
}

/** Prints the tests each leg selects, and exits unless each test is in exactly one leg. */
function checkCover(binaries, all, byLeg) {
  const problems = coverProblems(all, byLeg);
  console.log(`${binaries.length} test binaries, ${testCount(all)} tests`);
  for (const leg of LEGS) console.log(`  ${leg.id}: ${testCount(byLeg.get(leg.id))} tests (${leg.name})`);
  if (problems.length > 0) {
    console.error(`Each test must be in exactly one leg (scripts/check/ci/native-test-legs.mjs LEGS):\n${problems.join("\n")}`);
    process.exit(1);
  }
}

/** Writes the manifest and packs it with the binaries it names into `<dir>/native-tests.tar.zst`; returns the manifest. */
function pack(dir, binaries, all, byLeg) {
  // A binary with no test to run (cargo's empty harnesses) stays behind.
  const manifest = {
    binaries: binaries.filter((binary) => all.get(binary.exe).size > 0).map((binary) => ({
      exe: binary.exe,
      cwd: binary.cwd,
      env: binary.env,
      tests: Object.fromEntries(LEGS.map((leg) => [leg.id, byLeg.get(leg.id).get(binary.exe).size])),
    })),
  };
  writeFileSync(join(ROOT, "target", "native-tests.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  const files = ["target/native-tests.json", ...manifest.binaries.map((binary) => relative(ROOT, binary.exe))];
  // The binaries share most of their code (each links the dependencies): a 2 GB window finds it across files, 1.25 GB to
  // 0.79 GB for 44383070532's (level 3; level 9 saves 0.07 GB more at twice the time).
  const tar = spawnSync("tar", ["-I", `zstd -T0 -3 ${LONG}`, "-cf", join(dir, "native-tests.tar.zst"), ...files], { cwd: ROOT, stdio: "inherit" });
  if (tar.status !== 0) process.exit(tar.status ?? 1);
  return manifest;
}

/** Writes the `rust` job's matrix, the legs with a test to run, to $GITHUB_OUTPUT when there is one. */
function writeLegs(manifest) {
  const legs = LEGS.filter((leg) => manifest.binaries.some((binary) => binary.tests[leg.id] > 0)).map(({ id, name }) => ({ leg: id, name }));
  if (process.env.GITHUB_OUTPUT !== undefined) appendFileSync(process.env.GITHUB_OUTPUT, `legs=${JSON.stringify(legs)}\n`);
}

function build(dir) {
  const binaries = capturedBinaries(compileCaptured(dir));
  const all = new Map(binaries.map((binary) => [binary.exe, list(binary, [])]));
  const byLeg = new Map(LEGS.map((leg) => [leg.id, legTests(leg, binaries)]));
  checkCover(binaries, all, byLeg);
  writeLegs(pack(dir, binaries, all, byLeg));
}

function runBinary(binary, args) {
  const env = { ...process.env, ...binary.env };
  const extra = [];
  if (process.env.NATIVE_TESTS_REPORT_TIME === "1") {
    env.RUSTC_BOOTSTRAP = "1";
    extra.push("-Zunstable-options", "--report-time");
  }
  return new Promise((done) => {
    const child = spawn(binary.exe, [...args, ...extra], { cwd: binary.cwd, env, stdio: ["ignore", "pipe", "inherit"] });
    let last;
    let pending = "";
    child.stdout.on("data", (chunk) => {
      process.stdout.write(chunk);
      pending += chunk.toString("utf8");
      const lines = pending.split("\n");
      pending = lines.pop();
      for (const line of lines) last = parseResult(line) ?? last;
    });
    child.on("close", (code, signal) => {
      last = parseResult(pending) ?? last;
      done({ code, signal, result: last });
    });
  });
}

/** The leg `id` names; throws, listing the legs, when there is none. */
function findLeg(id) {
  const leg = LEGS.find((candidate) => candidate.id === id);
  if (leg === undefined) throw new Error(`no leg ${id}: ${LEGS.map((candidate) => candidate.id).join(", ")}`);
  return leg;
}

/** Unpacks the artifact in `dir` into the checkout; returns the binaries its manifest names. */
function unpack(dir) {
  const tar = spawnSync("tar", ["-I", `zstd -d ${LONG}`, "-xf", join(dir, "native-tests.tar.zst")], { cwd: ROOT, stdio: "inherit" });
  if (tar.status !== 0) process.exit(tar.status ?? 1);
  return JSON.parse(readFileSync(join(ROOT, "target", "native-tests.json"), "utf8")).binaries;
}

/** Adds a binary's summary counts, when it printed one, to the leg's totals. */
function addTotals(totals, result) {
  if (result === undefined) return;
  for (const key of Object.keys(totals)) totals[key] += result[key];
}

/** How a binary's run failed: its exit, or a test count other than the `expected` the build listed; else undefined. */
function binaryFailure(binary, expected, { code, signal, result }) {
  const name = relative(ROOT, binary.exe);
  if (code !== 0) return `${name}: ${signal === null ? `exit status ${code}` : `signal ${signal}`}`;
  if (result !== undefined && result.passed + result.failed + result.ignored + result.measured === expected) return undefined;
  return `${name}: ran ${result === undefined ? "no" : result.passed + result.failed + result.ignored} tests, the build listed ${expected}`;
}

/** Runs each binary with a test in `leg`, in turn; returns the failures and the summed counts. */
async function runLeg(leg, binaries) {
  const args = legArgs(leg);
  const failures = [];
  const totals = { passed: 0, failed: 0, ignored: 0 };
  for (const binary of binaries.filter((candidate) => candidate.tests[leg.id] > 0)) {
    console.log(`     Running ${relative(ROOT, binary.exe)} (${relative(ROOT, binary.cwd) || "."})`);
    const outcome = await runBinary(binary, args);
    addTotals(totals, outcome.result);
    const failure = binaryFailure(binary, binary.tests[leg.id], outcome);
    if (failure !== undefined) failures.push(failure);
  }
  return { failures, totals };
}

async function run(id, dir) {
  const leg = findLeg(id);
  const { failures, totals } = await runLeg(leg, unpack(dir));
  console.log(`\n${leg.name}: ${totals.passed} passed; ${totals.failed} failed; ${totals.ignored} ignored`);
  if (failures.length > 0) {
    console.error(`\nFailed:\n${failures.join("\n")}`);
    process.exit(1);
  }
}

async function main([command, ...rest]) {
  if (command === "capture") capture();
  else if (command === "build" && rest.length === 1) build(resolve(rest[0]));
  else if (command === "run" && rest.length === 2) await run(rest[0], resolve(rest[1]));
  else {
    console.error("usage: native-test-legs.mjs build <dir> | run <leg> <dir>");
    process.exit(2);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === SELF) await main(process.argv.slice(2));
