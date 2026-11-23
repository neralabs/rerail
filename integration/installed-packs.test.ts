/**
 * Rule packs installed from npm, against the registry (docs/rule-packs.md, "Installing packs"). Every step runs the
 * built `rerail` as its own process, the way a coding agent runs it: set-up on a project outside this repository
 * through both entry points, `rerail setup --relay` (and `/rerail-start` in Claude Code, Cursor and Codex, which run
 * `rerail start --relay`) and `rerail start --json` (OMP and Pi), then `rerail packs` upgrading, downgrading and
 * removing a published pack while the repository's settings stand.
 *
 * The packs are restricted, so this reads them with the npm credentials in the environment: CI's `installed-packs`
 * job sets `RERAIL_INSTALLED_PACKS_NPM=1` with a read-only token. Without it the file is skipped. Each run gets its
 * own `HOME`, so no coding agent on this machine is connected, and keeps npm's own user config for the token.
 *
 * The core is this build, whose version can be one npm's `latest` does not have yet: from the merge of a Release pull
 * request to its promotion, main is that release, published (or to be published) under `next` beside the pack versions
 * released with it, while the packs on `latest` may still name ranges that exclude it. So the Rerail home is on the
 * update channel an install of this version is on (`channel`), and the pack versions the steps name are read
 * from the registry by the range each one publishes (`admittedVersions`), not pinned.
 */
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import semver from "semver";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { PACK_FALLBACK_TAG, UPDATE_STATE_FILE } from "../src/setup/index.js";

import { RULE_PACK_LIST } from "../src/engine/index.js";

const NPM = process.env.RERAIL_INSTALLED_PACKS_NPM === "1";
/** A registry round trip per pack, several per test. */
const NPM_TIMEOUT_MS = 300_000;
const JAVASCRIPT = "@neralabs/rerail-pack-javascript";
const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");

interface Manifest {
  version: string;
  peerDependencies?: Record<string, string>;
}

/** `npm view <args> --json`, with the npm credentials in the environment. */
function npmView(args: readonly string[]): unknown {
  return JSON.parse(execFileSync("npm", ["view", ...args, "--json"], { encoding: "utf8" })) as unknown;
}

/** This build's manifest: its version is the core every pack's range is checked against, as `rerail` reports it. */
const CORE = JSON.parse(readFileSync(join(PACKAGE_ROOT, "package.json"), "utf8")) as Manifest;

/** The published plain versions of `packageName` whose core range admits this build's core, oldest first. */
function admittedVersions(packageName: string): string[] {
  const viewed = npmView([`${packageName}@>=0.0.0`, "version", "peerDependencies"]);
  // npm answers one matching version as an object, several as an array.
  const published = (Array.isArray(viewed) ? viewed : [viewed]) as Manifest[];
  return published
    .filter(manifest => {
      const range = manifest.peerDependencies?.["@neralabs/rerail"];
      return range !== undefined && semver.satisfies(CORE.version, range, { includePrerelease: true });
    })
    .map(manifest => manifest.version)
    .sort(semver.compare);
}

/**
 * The update channel an install of this core is on: `latest` once core's `latest` dist-tag is its version, else `next`,
 * where every release publishes core and its packs first (docs/release.md, "Promoting").
 */
let channel: string;
/** The two newest published versions of the JavaScript pack that load in this core, the older first. Both list eslint-eqeqeq. */
let older: string;
let newer: string;
let root: string;
let home: string;
let repo: string;
let environment: NodeJS.ProcessEnv;

beforeEach(() => {
  root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-installed-packs-")));
  home = join(root, "rerail-home");
  const userHome = join(root, "user-home");
  mkdirSync(home);
  mkdirSync(userHome);
  // This build is the active Rerail, as after `npx @neralabs/rerail setup` installed it: set-up installs no core.
  symlinkSync(PACKAGE_ROOT, join(home, "current"));
  // Past set-up's one approval that Rerail may read sessions; `setup-json.test.ts` asks it.
  writeFileSync(join(home, "consent.json"), JSON.stringify({ sessions: { approvedAt: "2026-10-16T00:00:00.000Z" } }));
  // The channel set-up fetches packs under, as `rerail update --channel` records it.
  writeFileSync(join(home, UPDATE_STATE_FILE), JSON.stringify({ schemaVersion: 1, channel, current: { version: CORE.version, packageRoot: PACKAGE_ROOT } }));
  environment = {
    ...process.env,
    RERAIL_HOME: home,
    HOME: userHome,
    NPM_CONFIG_USERCONFIG: process.env.NPM_CONFIG_USERCONFIG ?? join(homedir(), ".npmrc"),
    BROWSER: "none",
    RERAIL_AUTOSTART: "0",
  };
  // A project outside this repository: TypeScript, with a lint config that turns eqeqeq on, which the pack starts off.
  repo = join(root, "shop");
  mkdirSync(join(repo, "src"), { recursive: true });
  writeFileSync(join(repo, "package.json"), JSON.stringify({ name: "shop", devDependencies: { typescript: "^5.9.0" } }));
  writeFileSync(join(repo, "src", "cart.ts"), "export function total(prices: number[]): number {\n  return prices.reduce((sum, price) => sum + price, 0);\n}\n");
  writeFileSync(join(repo, ".eslintrc.json"), JSON.stringify({ rules: { eqeqeq: "error" } }));
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
  execFileSync("git", ["add", "."], { cwd: repo });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "base"], { cwd: repo });
});

afterEach(async () => {
  if (existsSync(join(home, "daemon.json"))) await rerail(["daemon", "stop"]);
  rmSync(root, { recursive: true, force: true });
});

async function rerail(args: readonly string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  const child = spawn(process.execPath, [CLI, ...args], { cwd: repo, env: environment, stdio: ["ignore", "pipe", "pipe"] });
  const [stdout, stderr, code] = await Promise.all([
    text(child.stdout),
    text(child.stderr),
    new Promise<number>((done, fail) => {
      child.once("error", fail);
      child.once("close", exit => done(exit ?? 1));
    }),
  ]);
  return { code, stdout, stderr };
}

/** The arguments of the relayed answer command numbered `answer` (`1. rerail setup --relay --answer …`). */
function answerArgs(question: string, answer: number): string[] {
  const line = question.split("\n").find(candidate => candidate.startsWith(`${answer}. rerail `));
  if (line === undefined) throw new Error(`No answer ${answer} in:\n${question}`);
  return line.slice(`${answer}. rerail `.length).split(" ");
}

const installed = (): Record<string, string> =>
  (JSON.parse(readFileSync(join(home, "packs", "package.json"), "utf8")) as { dependencies: Record<string, string> }).dependencies;

/** The running daemon's answer to `path`, starting it if set-up did not. */
async function daemonGet<T>(path: string): Promise<T> {
  if (!existsSync(join(home, "daemon.json"))) await rerail(["daemon", "start"]);
  const { url, accessToken } = JSON.parse(readFileSync(join(home, "daemon.json"), "utf8")) as { url: string; accessToken: string };
  const response = await fetch(new URL(path, url), { headers: { "x-rerail-access": accessToken } });
  expect(response.status).toBe(200);
  return await response.json() as T;
}

/**
 * The listed packs this repository has never released and npm does not have: a new pack is listed when it merges and
 * published by hand right after (docs/publishing-a-new-pack.md), and until then set-up names it as one it could not
 * use. Never released is read from the repository alone: the Release pull request's `changeset version` takes a pack
 * off `0.1.0` (`npm run version:packs`) and `scripts/release/release-version.mjs` writes its first `CHANGELOG.md`
 * (`releasedPackChangelog`), so a pack still at `0.1.0` with no `CHANGELOG.md` was never released. Any other pack npm
 * lacks is not tolerated, and a never-released pack npm has must load like the rest.
 */
function unreleasedPacks(): string[] {
  return RULE_PACK_LIST.filter(pack => {
    const directory = join(PACKAGE_ROOT, "packs", pack.id);
    const { version } = JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as { version: string };
    return version === "0.1.0" && !existsSync(join(directory, "CHANGELOG.md")) && !onNpm(pack.packageName);
  }).map(pack => pack.packageName);
}

/** Whether npm has `name`: false only when npm answers E404 for it. */
function onNpm(name: string): boolean {
  try {
    execFileSync("npm", ["view", name, "name"], { env: environment, stdio: "pipe" });
    return true;
  } catch (error) {
    const stderr = error instanceof Error && "stderr" in error ? String(error.stderr) : "";
    if (/\bE404\b/u.test(stderr)) return false;
    throw error;
  }
}

/** The lines set-up prints under "Rule packs set-up could not use:", one per pack, in the order it fetched them. */
function unusable(stdout: string): string[] {
  const lines = stdout.split("\n");
  const heading = lines.indexOf("  Rule packs set-up could not use:");
  if (heading < 0) return [];
  const block = lines.slice(heading + 1);
  const end = block.findIndex(line => !line.startsWith("    "));
  return (end < 0 ? block : block.slice(0, end)).map(line => line.trim());
}

interface RepositoryRules {
  rules: { rule: { id: string }; on: boolean }[];
  unavailable: { sourceId: string; reason: string }[];
}

/** eslint-eqeqeq in this repository as its settings leave it, and the attachments the catalog cannot resolve. */
async function eqeqeq(): Promise<{ on: boolean | undefined; unavailable: string[] }> {
  const view = await daemonGet<RepositoryRules>(`/api/repository/rules?checkout=${encodeURIComponent(repo)}`);
  return {
    on: view.rules.find(entry => entry.rule.id === "eslint-eqeqeq")?.on,
    unavailable: view.unavailable.map(entry => `${entry.sourceId}: ${entry.reason}`),
  };
}

describe.skipIf(!NPM)("rule packs installed from npm", { timeout: NPM_TIMEOUT_MS }, () => {
  beforeAll(() => {
    channel = npmView(["@neralabs/rerail", "dist-tags.latest"]) === CORE.version ? "latest" : PACK_FALLBACK_TAG;
    const admitted = admittedVersions(JAVASCRIPT);
    // A release writes a pack's range open above (`>=<version>-0`), so each release's pack still loads in the next core:
    // only the capped ranges (`<0.2.0`) of the packs published before 0.2.0 could leave fewer than two.
    if (admitted.length < 2) throw new Error(`Upgrading and downgrading ${JAVASCRIPT} needs two published versions whose core range admits Rerail ${CORE.version}; npm has ${admitted.length === 0 ? "none" : admitted[0]}`);
    older = admitted.at(-2)!;
    newer = admitted.at(-1)!;
  });

  it("`rerail setup --relay`: shows the published packs that fit an outside project, installs nothing before the answer, then exactly the versions the answer carries", async () => {
    const read = await rerail(["setup", "--relay"]);
    expect(read.stdout).toContain("Find rule packs for shop?");
    expect(existsSync(join(home, "packs"))).toBe(false);

    const confirm = await rerail(answerArgs(read.stdout, 1));
    // A TypeScript project: the TypeScript pack fits its dependency, complexity its *.ts source, and JavaScript its
    // source too, holding eslint-eqeqeq, which the pack starts off and the lint config turns on. Every published pack
    // loads; a listed pack never released yet (unreleasedPacks) is named as not published.
    for (const id of ["javascript", "typescript", "complexity"]) expect(confirm.stdout).toMatch(new RegExp(`--packs \\S*\\b${id}@\\d+\\.\\d+\\.\\d+`, "u"));
    expect(unusable(confirm.stdout)).toEqual(unreleasedPacks().map(name => expect.stringContaining(`npm cannot read ${name}: it is not published`)));
    expect(confirm.stdout).toContain("eslint-eqeqeq starts on");
    expect(existsSync(join(home, "packs", "package.json"))).toBe(false);

    // The answer carries exact versions: one other than the tag gives is installed as named, not re-resolved.
    const start = answerArgs(confirm.stdout, 1);
    const packs = start.indexOf("--packs") + 1;
    const named = /javascript@([^,]+)/u.exec(start[packs]!)?.[1] === newer ? older : newer;
    start[packs] = start[packs]!.replace(/javascript@[^,]+/u, `javascript@${named}`);
    const started = await rerail(start);
    expect(started.stdout).toContain("Rerail is on in shop ·");
    // Every other pack is installed at the version the answer carried.
    const carried = Object.fromEntries(start[packs]!.split(",").map(spec => [`@neralabs/rerail-pack-${spec.slice(0, spec.lastIndexOf("@"))}`, spec.slice(spec.lastIndexOf("@") + 1)]));
    expect(installed()).toEqual(carried);
    expect(installed()[JAVASCRIPT]).toBe(named);

    const listed = await daemonGet<{ packs: { id: string; version: string | null }[]; skipped: unknown[] }>("/api/rule-packs");
    expect(listed.packs.map(found => found.id).sort()).toEqual(["complexity", "javascript", "typescript"]);
    expect(listed.skipped).toEqual([]);
    const rerailRules = await daemonGet<{ rules: { rule: { id: string }; packId: string | null }[] }>("/api/rerail-rules");
    expect(rerailRules.rules.find(entry => entry.rule.id === "eslint-eqeqeq")?.packId).toBe("javascript");
    expect(await eqeqeq()).toEqual({ on: true, unavailable: [] });
    expect((await rerail(["state"])).stdout.trim()).toBe("started");

    // In a repository that is set up, `/rerail-start` just starts it: nothing is fetched or installed.
    rmSync(join(home, "packs", "staging"), { recursive: true, force: true });
    const before = readFileSync(join(home, "packs", "package.json"), "utf8");
    const again = await rerail(["start", "--relay"]);
    expect(again.code).toBe(0);
    expect(existsSync(join(home, "packs", "staging"))).toBe(false);
    expect(readFileSync(join(home, "packs", "package.json"), "utf8")).toBe(before);
  });

  it("`rerail start --json`: OMP's and Pi's steps install the packs the confirm question showed", async () => {
    interface Step { step: string; question: { options: { answer: string; next: { args: string[] } }[] } | null }
    const step = async (args: readonly string[]): Promise<Step> => JSON.parse((await rerail(args)).stdout) as Step;
    const option = (from: Step, answer: string): string[] => from.question!.options.find(candidate => candidate.answer === answer)!.next.args;

    const read = await step(["start", "--json"]);
    expect(read.step).toBe("read-repo");
    const confirm = await step(option(read, "yes"));
    expect(confirm.step).toBe("confirm-packs");
    const startArgs = option(confirm, "start");
    const shown = startArgs[startArgs.indexOf("--packs") + 1]!;
    expect(existsSync(join(home, "packs", "package.json"))).toBe(false);

    expect((await step(startArgs)).step).toBe("done");
    expect(Object.entries(installed()).map(([name, version]) => `${name.slice("@neralabs/rerail-pack-".length)}@${version}`).sort())
      .toEqual(shown.split(",").sort());
    expect(await eqeqeq()).toEqual({ on: true, unavailable: [] });
  });

  it("`rerail packs` upgrades, downgrades and removes a published pack while the repository's settings stand, and leaves out one whose core range excludes Rerail", async () => {
    expect((await rerail(["packs", "add", `javascript@${older}`])).stdout).toContain(`javascript@${older}`);
    const read = await rerail(["setup", "--relay"]);
    const confirm = await rerail(answerArgs(read.stdout, 1));
    await rerail(answerArgs(confirm.stdout, 1));
    expect(await eqeqeq()).toEqual({ on: true, unavailable: [] });

    expect((await rerail(["packs", "add", `javascript@${newer}`])).stdout).toContain(`javascript@${newer}`);
    expect(installed()["@neralabs/rerail-pack-javascript"]).toBe(newer);
    expect(await eqeqeq()).toEqual({ on: true, unavailable: [] });

    expect((await rerail(["packs", "add", `javascript@${older}`])).stdout).toContain(`javascript@${older}`);
    expect(installed()["@neralabs/rerail-pack-javascript"]).toBe(older);
    expect(await eqeqeq()).toEqual({ on: true, unavailable: [] });

    // npm cannot supply a pack whose range excludes this core, so narrow an installed one.
    const manifest = join(home, "packs", "node_modules", "@neralabs", "rerail-pack-javascript", "package.json");
    const original = readFileSync(manifest, "utf8");
    writeFileSync(manifest, JSON.stringify({ ...JSON.parse(original) as object, peerDependencies: { "@neralabs/rerail": ">=9.0.0" } }));
    expect((await rerail(["packs", "list"])).stdout).toMatch(new RegExp(`left out: ${JAVASCRIPT}@${older.replaceAll(".", "\\.")}: .*>=9\\.0\\.0`, "u"));
    expect((await eqeqeq()).unavailable).toContain("javascript: missing");
    writeFileSync(manifest, original);

    const removed = await rerail(["packs", "remove", "javascript"]);
    expect(removed.code).toBe(0);
    expect(await eqeqeq()).toEqual({ on: undefined, unavailable: ["javascript: missing"] });
    // Installing it again restores what the repository had.
    await rerail(["packs", "add", `javascript@${newer}`]);
    expect(await eqeqeq()).toEqual({ on: true, unavailable: [] });
  });
});
