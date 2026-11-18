#!/usr/bin/env node
// The engine's Rust tests in CI: compiled once, run in legs (ci.yml's `rust-tests-build` and `rust` jobs).
//
// `build` runs `cargo test --workspace` with this script as cargo's target runner (`capture`): cargo compiles every
// test binary and runs the doc tests (rustdoc hands them to the runner, which runs them), and hands each test binary to
// the runner, which records it with the working directory and the environment cargo gave it (the package's directory,
// CARGO_MANIFEST_DIR, CARGO_PKG_*, .cargo/config.toml's [env], the library path) instead of running it. Then it lists every binary's tests
// (`--list`) whole and under each leg's filters, and fails unless each test falls in exactly one leg. What it writes
// (`<dir>/native-tests.tar.zst`: the binaries at their paths under target/, and the manifest) is the artifact each
// leg unpacks in its checkout, at the same path as the build's, where `run <leg>` runs each binary with the leg's
// libtest filters as `cargo test --workspace -- <filters>` would: same working directory, same environment (the leg's
// own PATH kept), libtest as the harness. Unlike cargo, a failed binary does not stop the ones after it.
//
// The legs are libtest filters over the test paths, so a test's leg follows its module: the last leg skips what the
// others select, and the build's check keeps "every test in exactly one leg" true as tests move.
//
//   node scripts/check/native-test-legs.mjs build <dir>    compile, run the doc tests, check the legs, pack
//   node scripts/check/native-test-legs.mjs run <leg> <dir> run one leg's tests from the unpacked artifact
//   node scripts/check/native-test-legs.mjs legs            the legs, as the `rust` job's matrix (JSON)
//
// NATIVE_TESTS_REPORT_TIME=1 has `run` add libtest's unstable `--report-time` (under RUSTC_BOOTSTRAP=1), each test's
// time, to rebalance the legs from a run's log.
import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const SELF = fileURLToPath(import.meta.url);

/** One leg: the tests whose path holds any of `filters` (every test when empty) and none of `skip`. */
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
];

/** The legs, in order; the last runs every test the others' filters do not name. */
export const LEGS = [
  ...SELECTED,
  { id: "rest", name: "tests, the rest", filters: [], skip: SELECTED.flatMap((leg) => leg.filters) },
];

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

function build(dir) {
  mkdirSync(dir, { recursive: true });
  const captured = join(dir, "captured.jsonl");
  const parentEnv = join(dir, "parent-env.json");
  writeFileSync(captured, "");
  const runner = `CARGO_TARGET_${hostTriple().toUpperCase().replaceAll("-", "_")}_RUNNER`;
  const env = { ...process.env, [runner]: `node ${SELF} capture`, NATIVE_TESTS_CAPTURE: captured, NATIVE_TESTS_PARENT_ENV: parentEnv };
  writeFileSync(parentEnv, JSON.stringify(env));
  const started = Date.now();
  // `npm run test:native`'s command.
  const cargo = spawnSync("cargo", ["test", "--manifest-path", "Cargo.toml", "--workspace"], { cwd: ROOT, stdio: "inherit", env });
  if (cargo.status !== 0) process.exit(cargo.status ?? 1);
  console.log(`cargo test (compile, doc tests): ${((Date.now() - started) / 60000).toFixed(1)} min`);

  const binaries = readFileSync(captured, "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line));
  for (const binary of binaries) {
    if (binary.args.length > 0) throw new Error(`cargo passed ${binary.exe} arguments: ${binary.args.join(" ")}`);
    if (!binary.path) throw new Error(`cargo changed PATH for ${binary.exe}; run keeps the leg's own`);
    if (!isAbsolute(binary.exe)) binary.exe = resolve(binary.cwd, binary.exe);
  }
  const all = new Map(binaries.map((binary) => [binary.exe, list(binary, [])]));
  const byLeg = new Map(LEGS.map((leg) => [leg.id, new Map(binaries.map((binary) => [binary.exe, list(binary, legArgs(leg))]))]));
  const problems = coverProblems(all, byLeg);
  const total = [...all.values()].reduce((sum, tests) => sum + tests.size, 0);
  console.log(`${binaries.length} test binaries, ${total} tests`);
  for (const leg of LEGS) {
    const count = [...byLeg.get(leg.id).values()].reduce((sum, tests) => sum + tests.size, 0);
    console.log(`  ${leg.id}: ${count} tests (${leg.name})`);
  }
  if (problems.length > 0) {
    console.error(`Each test must be in exactly one leg (scripts/check/native-test-legs.mjs LEGS):\n${problems.join("\n")}`);
    process.exit(1);
  }

  const manifest = {
    binaries: binaries.map((binary) => ({
      exe: binary.exe,
      cwd: binary.cwd,
      env: binary.env,
      tests: Object.fromEntries(LEGS.map((leg) => [leg.id, byLeg.get(leg.id).get(binary.exe).size])),
    })),
  };
  writeFileSync(join(ROOT, "target", "native-tests.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  const files = ["target/native-tests.json", ...binaries.map((binary) => relative(ROOT, binary.exe))];
  const tar = spawnSync("tar", ["-I", "zstd -T0 -3", "-cf", join(dir, "native-tests.tar.zst"), ...files], { cwd: ROOT, stdio: "inherit" });
  if (tar.status !== 0) process.exit(tar.status ?? 1);
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

async function run(id, dir) {
  const leg = LEGS.find((candidate) => candidate.id === id);
  if (leg === undefined) throw new Error(`no leg ${id}: ${LEGS.map((candidate) => candidate.id).join(", ")}`);
  const tar = spawnSync("tar", ["-I", "zstd", "-xf", join(dir, "native-tests.tar.zst")], { cwd: ROOT, stdio: "inherit" });
  if (tar.status !== 0) process.exit(tar.status ?? 1);
  const { binaries } = JSON.parse(readFileSync(join(ROOT, "target", "native-tests.json"), "utf8"));
  const args = legArgs(leg);
  const failures = [];
  const totals = { passed: 0, failed: 0, ignored: 0 };
  for (const binary of binaries.filter((candidate) => candidate.tests[leg.id] > 0)) {
    console.log(`     Running ${relative(ROOT, binary.exe)} (${relative(ROOT, binary.cwd) || "."})`);
    const { code, signal, result } = await runBinary(binary, args);
    if (result !== undefined) for (const key of Object.keys(totals)) totals[key] += result[key];
    if (code !== 0) failures.push(`${relative(ROOT, binary.exe)}: ${signal === null ? `exit status ${code}` : `signal ${signal}`}`);
    else if (result === undefined || result.passed + result.failed + result.ignored + result.measured !== binary.tests[leg.id]) {
      failures.push(`${relative(ROOT, binary.exe)}: ran ${result === undefined ? "no" : result.passed + result.failed + result.ignored} tests, the build listed ${binary.tests[leg.id]}`);
    }
  }
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
  else if (command === "legs" && rest.length === 0) console.log(JSON.stringify(LEGS.map(({ id, name }) => ({ leg: id, name }))));
  else {
    console.error("usage: native-test-legs.mjs build <dir> | run <leg> <dir> | legs");
    process.exit(2);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === SELF) await main(process.argv.slice(2));
