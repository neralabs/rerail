import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { constants as osConstants, tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "vitest";

import {
  accessFailureMessage,
  classifyNpmError,
  exitCodeFor,
  normalizeRegistry,
  npmEnvironment,
  npmErrorReason,
  parseSetupArguments,
  platformCommand,
  probeAccess,
  validateRegistry,
  validateTag,
} from "./rerail-setup.mjs";

const SCRIPT = fileURLToPath(new URL("./rerail-setup.mjs", import.meta.url));

function withTempDir(body) {
  const dir = mkdtempSync(join(tmpdir(), "rerail-setup-test-"));
  try {
    return body(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/**
 * Fake npm and npx on PATH: npm answers `view` and `whoami` from the given exit codes and output, npx records its
 * arguments and the scope registry it was handed. Nothing reaches a registry or the real ~/.npmrc.
 */
function stubTools(dir, { view, whoami }) {
  const bin = join(dir, "bin");
  mkdirSync(bin);
  const npxLog = join(dir, "npx-calls");
  writeFileSync(join(bin, "npm"), [
    "#!/bin/sh",
    'case "$1" in',
    `  view) printf '%s\\n' ${JSON.stringify(view.output)}; exit ${view.status} ;;`,
    `  whoami) printf '%s\\n' ${JSON.stringify(whoami.output)}; exit ${whoami.status} ;;`,
    "  config) echo undefined; exit 0 ;;",
    "esac",
    "exit 1",
    "",
  ].join("\n"), { mode: 0o755 });
  // A Node stub, not sh: dash (Linux /bin/sh) drops `npm_config_@neralabs:registry` because the name is not a
  // valid shell identifier, while npm and npx (Node) receive it.
  writeFileSync(join(bin, "npx"), [
    `#!${process.execPath}`,
    'const { appendFileSync } = require("node:fs");',
    `appendFileSync(${JSON.stringify(npxLog)}, \`\${process.argv.slice(2).join(" ")}\\nregistry=\${process.env["npm_config_@neralabs:registry"] ?? ""}\\n\`);`,
    "",
  ].join("\n"), { mode: 0o755 });
  const env = { PATH: `${bin}:${process.env.PATH}`, HOME: dir, NPM_CONFIG_USERCONFIG: join(dir, "npmrc") };
  return { env, npxLog };
}

function runScript(args, env) {
  return spawnSync(process.execPath, [SCRIPT, ...args], { env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 60_000 });
}

test("normalizeRegistry adds exactly one trailing slash", () => {
  assert.equal(normalizeRegistry("http://127.0.0.1:4873"), "http://127.0.0.1:4873/");
  assert.equal(normalizeRegistry("http://127.0.0.1:4873//"), "http://127.0.0.1:4873/");
});

test("validateRegistry accepts https anywhere and http only on loopback", () => {
  assert.deepEqual(validateRegistry("https://registry.npmjs.org"), { registry: "https://registry.npmjs.org/" });
  assert.deepEqual(validateRegistry("http://127.0.0.1:4873"), { registry: "http://127.0.0.1:4873/" });
  assert.deepEqual(validateRegistry("http://localhost:4873/"), { registry: "http://localhost:4873/" });
  assert.deepEqual(validateRegistry("http://[::1]:4873/"), { registry: "http://[::1]:4873/" });
  for (const bad of ["http://example.com/", "ftp://example.com/", "not a url", "https://example.com/a b/", "https://example.com/\nx=y", "https://u:p@example.com/", ""]) {
    const result = validateRegistry(bad);
    assert.ok("error" in result, bad);
    assert.match(result.error, /RERAIL_REGISTRY/u, bad);
  }
});

test("tags are dist-tags or versions, never paths or flags", () => {
  for (const good of ["latest", "next", "0.1.0", "0.1.0-next.3", "1.0.0+build"]) assert.deepEqual(validateTag(good), { tag: good });
  for (const bad of ["", "-x", "../x", "a b", "a/b", "x".repeat(65)]) assert.ok("error" in validateTag(bad), bad);
});

test("--tag beats RERAIL_TAG beats latest; --restore and --tag never reach rerail setup", () => {
  withTempDir(dir => {
    mkdirSync(join(dir, ".rerail-clean-20260929", "rerail-home"), { recursive: true });
    const env = { HOME: dir };
    assert.deepEqual(parseSetupArguments(["--project", "p"], { env }), { help: false, args: ["--project", "p"], tag: "latest", backupDir: undefined });
    assert.equal(parseSetupArguments([], { env: { ...env, RERAIL_TAG: "next" } }).tag, "next");
    assert.equal(parseSetupArguments(["--tag", "0.1.0"], { env: { ...env, RERAIL_TAG: "next" } }).tag, "0.1.0");
    assert.equal(parseSetupArguments(["--tag=next"], { env }).tag, "next");
    assert.deepEqual(
      parseSetupArguments(["--restore", "--tag", "next", "--port", "1"], { env }),
      { help: false, args: ["--port", "1"], tag: "next", backupDir: join(dir, ".rerail-clean-20260929") },
    );
    assert.throws(() => parseSetupArguments(["--tag"], { env }), /--tag needs/u);
    assert.throws(() => parseSetupArguments(["--tag", "a", "--tag", "b"], { env }), /once/u);
    assert.throws(() => parseSetupArguments([], { env: { ...env, RERAIL_TAG: "../x" } }), /RERAIL_TAG/u);
    assert.equal(parseSetupArguments(["--port", "1", "--help"], { env }).help, true);
  });
});

test("npmErrorReason prefers the npm error code, then the first line", () => {
  assert.equal(npmErrorReason("npm error code ECONNREFUSED\nnpm error errno ECONNREFUSED\n"), "ECONNREFUSED");
  assert.equal(npmErrorReason("\n  something odd happened  \nmore"), "something odd happened");
  assert.equal(npmErrorReason("x".repeat(300)).length, 120);
  assert.equal(npmErrorReason(""), "no response");
});

test("npm error classification", () => {
  assert.equal(classifyNpmError("npm error code E401\nnpm error Unable to authenticate"), "auth");
  assert.equal(classifyNpmError("npm error code ENEEDAUTH"), "auth");
  assert.equal(classifyNpmError("npm error code E403"), "auth");
  assert.equal(classifyNpmError("npm error code E404\nnpm error 404 Not Found - GET https://registry.npmjs.org/@neralabs%2frerail"), "not-found");
  assert.equal(classifyNpmError("npm error code ECONNREFUSED"), "network");
  assert.equal(classifyNpmError("weird"), "other");
});

test("an unreadable package asks for npm login when logged out, and for scope access when logged in", () => {
  const base = { kind: "not-found", reason: "E404", registry: "https://registry.npmjs.org/", tag: "latest" };
  const loggedOut = accessFailureMessage({ ...base, user: undefined });
  assert.match(loggedOut, /npm is not logged in to https:\/\/registry\.npmjs\.org\//u);
  assert.match(loggedOut, /`npm login`/u);
  assert.match(loggedOut, /@neralabs scope/u);
  const noAccess = accessFailureMessage({ ...base, kind: "auth", reason: "E403", user: "alice" });
  assert.match(noAccess, /logged in .* as alice, but alice cannot read @neralabs\/rerail@latest \(E403\)/u);
  assert.match(noAccess, /organization owner to give alice read access/u);
  assert.match(accessFailureMessage({ ...base, kind: "network", reason: "ECONNREFUSED" }), /Could not reach .*ECONNREFUSED/u);
});

test("probeAccess asks who npm is logged in as only after an auth or not-found failure", () => {
  const calls = [];
  const runner = answers => (args) => {
    calls.push(args[0] === "config" ? `config ${args[2]}` : args[0]);
    return answers[args[0]] ?? { ok: false, stdout: "", output: "" };
  };
  assert.deepEqual(probeAccess({ tag: "latest", env: {}, run: runner({ view: { ok: true, stdout: "0.1.0", output: "0.1.0" } }) }), { ok: true, version: "0.1.0" });
  assert.deepEqual(calls, ["view"]);
  calls.length = 0;
  const denied = probeAccess({
    tag: "next",
    env: {},
    run: runner({
      view: { ok: false, stdout: "", output: "npm error code E404\n" },
      whoami: { ok: true, stdout: "alice", output: "alice" },
      config: { ok: true, stdout: "http://127.0.0.1:9/", output: "" },
    }),
  });
  assert.deepEqual(denied, { ok: false, kind: "not-found", reason: "E404", registry: "http://127.0.0.1:9/", user: "alice" });
  assert.deepEqual(calls, ["view", "config @neralabs:registry", "whoami"]);
  assert.deepEqual(probeAccess({ tag: "x", env: {}, run: runner({ view: { ok: true, stdout: "", output: "" } }) }), { ok: false, kind: "no-tag" });
});

test("RERAIL_REGISTRY reaches every npm child as the @neralabs scope registry", () => {
  assert.deepEqual(npmEnvironment({ A: "1" }, undefined), { A: "1" });
  assert.deepEqual(npmEnvironment({ A: "1" }, "http://127.0.0.1:9/"), { A: "1", "npm_config_@neralabs:registry": "http://127.0.0.1:9/" });
});

test("POSIX spawns npm and npx by name; Windows runs them through node with no shell", () => {
  assert.deepEqual(platformCommand("npx", ["--yes", "a&b"], { platform: "linux", execPath: "/n/node", env: {} }), { command: "npx", args: ["--yes", "a&b"] });
  const execPath = "C:\\Program Files\\nodejs\\node.exe";
  const hostile = ["--project", "a & calc.exe", "\"quoted\""];
  const npx = platformCommand("npx", ["--yes", "@neralabs/rerail@latest", "setup", ...hostile], { platform: "win32", execPath, env: {} });
  assert.equal(npx.command, execPath);
  assert.match(npx.args[0], /node_modules[\\/]npm[\\/]bin[\\/]npx-cli\.js$/u);
  assert.deepEqual(npx.args.slice(-3), hostile);
});

test("a child killed by a signal exits 128 + that signal's number", () => {
  assert.equal(exitCodeFor(0, null), 0);
  assert.equal(exitCodeFor(3, null), 3);
  assert.equal(exitCodeFor(null, "SIGINT"), 128 + osConstants.signals.SIGINT);
  assert.equal(exitCodeFor(null, null), 1);
});

test("--help prints the usage and runs nothing", { skip: process.platform === "win32" }, () => {
  withTempDir(dir => {
    const { env, npxLog } = stubTools(dir, { view: { status: 0, output: "0.1.0" }, whoami: { status: 0, output: "alice" } });
    const result = runScript(["--help"], env);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /usage: just rerail \[--tag <tag>\] \[--restore \[backup-dir\]\]/u);
    assert.throws(() => readFileSync(npxLog));
  });
});

test("a logged-out npm stops before npx with a message to run npm login", { skip: process.platform === "win32" }, () => {
  withTempDir(dir => {
    const { env, npxLog } = stubTools(dir, {
      view: { status: 1, output: "npm error code E404" },
      whoami: { status: 1, output: "npm error code ENEEDAUTH" },
    });
    const result = runScript([], env);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /npm is not logged in to .*Run `npm login`/su);
    assert.throws(() => readFileSync(npxLog));
  });
});

test("an accessible package hands the tag, the passthrough arguments and RERAIL_REGISTRY to npx setup", { skip: process.platform === "win32" }, () => {
  withTempDir(dir => {
    const { env, npxLog } = stubTools(dir, { view: { status: 0, output: "0.2.0-next.1" }, whoami: { status: 0, output: "alice" } });
    const result = runScript(["--tag", "next", "--port", "4800"], { ...env, RERAIL_REGISTRY: "http://127.0.0.1:9" });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /@neralabs\/rerail@next is 0\.2\.0-next\.1/u);
    assert.equal(readFileSync(npxLog, "utf8"), "--yes @neralabs/rerail@next setup --port 4800\nregistry=http://127.0.0.1:9/\n");
  });
});

test("an invalid RERAIL_REGISTRY exits non-zero before touching npm", { skip: process.platform === "win32" }, () => {
  withTempDir(dir => {
    const { env, npxLog } = stubTools(dir, { view: { status: 0, output: "0.1.0" }, whoami: { status: 0, output: "alice" } });
    const result = runScript([], { ...env, RERAIL_REGISTRY: "http://example.com/" });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /RERAIL_REGISTRY/u);
    assert.doesNotMatch(result.stdout, /Checking npm access/u);
    assert.throws(() => readFileSync(npxLog));
  });
});

test("just rerail --restore without a backup fails before touching npm", { skip: process.platform === "win32" }, () => {
  withTempDir(dir => {
    const { env, npxLog } = stubTools(dir, { view: { status: 0, output: "0.1.0" }, whoami: { status: 0, output: "alice" } });
    const result = runScript(["--restore"], env);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /No Rerail backup found/u);
    assert.doesNotMatch(result.stdout, /Checking npm access/u);
    assert.throws(() => readFileSync(npxLog));
  });
});
