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
 */
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const NPM = process.env.RERAIL_INSTALLED_PACKS_NPM === "1";
/** A registry round trip per pack, several per test. */
const NPM_TIMEOUT_MS = 300_000;
/** Two published versions of the JavaScript pack, the older first. Both list eslint-eqeqeq. */
const JAVASCRIPT_VERSIONS = ["0.1.1", "0.1.2"] as const;
const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");

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
  environment = {
    ...process.env,
    RERAIL_HOME: home,
    HOME: userHome,
    NPM_CONFIG_USERCONFIG: process.env.NPM_CONFIG_USERCONFIG ?? join(homedir(), ".npmrc"),
    BROWSER: "none",
    RERAIL_AUTOSTART: "0",
  };
  // A project outside this repository: TypeScript, with a lint config that turns eqeqeq off.
  repo = join(root, "shop");
  mkdirSync(join(repo, "src"), { recursive: true });
  writeFileSync(join(repo, "package.json"), JSON.stringify({ name: "shop", devDependencies: { typescript: "^5.9.0" } }));
  writeFileSync(join(repo, "src", "cart.ts"), "export function total(prices: number[]): number {\n  return prices.reduce((sum, price) => sum + price, 0);\n}\n");
  writeFileSync(join(repo, ".eslintrc.json"), JSON.stringify({ rules: { eqeqeq: "off" } }));
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
  const { url } = JSON.parse(readFileSync(join(home, "daemon.json"), "utf8")) as { url: string };
  const response = await fetch(new URL(path, url));
  expect(response.status).toBe(200);
  return await response.json() as T;
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
  it("`rerail setup --relay`: shows the published packs that fit an outside project, installs nothing before the answer, then exactly the versions the answer carries", async () => {
    const read = await rerail(["setup", "--relay"]);
    expect(read.stdout).toContain("Can Rerail read it to pick the rule packs that look right?");
    expect(existsSync(join(home, "packs"))).toBe(false);

    const confirm = await rerail(answerArgs(read.stdout, 1));
    // JavaScript fits a TypeScript-only project and holds eslint-eqeqeq, which the lint config turns off. Packs
    // published before `match` was required are named among those set-up could not use, not offered.
    expect(confirm.stdout).toMatch(/@neralabs\/rerail-pack-javascript@\d+\.\d+\.\d+/u);
    expect(confirm.stdout).toContain("eslint-eqeqeq starts off");
    expect(existsSync(join(home, "packs", "package.json"))).toBe(false);

    // The answer carries exact versions: an older one than the tag gives is installed as named, not re-resolved.
    const start = answerArgs(confirm.stdout, 1);
    const packs = start.indexOf("--packs") + 1;
    start[packs] = start[packs]!.replace(/javascript@[^,]+/u, `javascript@${JAVASCRIPT_VERSIONS[0]}`);
    const started = await rerail(start);
    expect(started.stdout).toContain("Rerail is on in shop.");
    expect(installed()["@neralabs/rerail-pack-javascript"]).toBe(JAVASCRIPT_VERSIONS[0]);

    const listed = await daemonGet<{ packs: { id: string; version: string | null }[]; skipped: unknown[] }>("/api/rule-packs");
    expect(listed.packs).toContainEqual(expect.objectContaining({ id: "javascript", version: JAVASCRIPT_VERSIONS[0] }));
    expect(listed.skipped).toEqual([]);
    const rerailRules = await daemonGet<{ rules: { rule: { id: string }; packId: string | null }[] }>("/api/rerail-rules");
    expect(rerailRules.rules.find(entry => entry.rule.id === "eslint-eqeqeq")?.packId).toBe("javascript");
    expect(await eqeqeq()).toEqual({ on: false, unavailable: [] });
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
    expect(await eqeqeq()).toEqual({ on: false, unavailable: [] });
  });

  it("`rerail packs` upgrades, downgrades and removes a published pack while the repository's settings stand, and leaves out one whose core range excludes Rerail", async () => {
    const [older, newer] = JAVASCRIPT_VERSIONS;
    expect((await rerail(["packs", "add", `javascript@${older}`])).stdout).toContain(`javascript@${older}`);
    const read = await rerail(["setup", "--relay"]);
    const confirm = await rerail(answerArgs(read.stdout, 1));
    await rerail(answerArgs(confirm.stdout, 1));
    expect(await eqeqeq()).toEqual({ on: false, unavailable: [] });

    expect((await rerail(["packs", "add", `javascript@${newer}`])).stdout).toContain(`javascript@${newer}`);
    expect(installed()["@neralabs/rerail-pack-javascript"]).toBe(newer);
    expect(await eqeqeq()).toEqual({ on: false, unavailable: [] });

    expect((await rerail(["packs", "add", `javascript@${older}`])).stdout).toContain(`javascript@${older}`);
    expect(installed()["@neralabs/rerail-pack-javascript"]).toBe(older);
    expect(await eqeqeq()).toEqual({ on: false, unavailable: [] });

    // npm cannot supply a pack whose range excludes this core, so narrow an installed one.
    const manifest = join(home, "packs", "node_modules", "@neralabs", "rerail-pack-javascript", "package.json");
    const original = readFileSync(manifest, "utf8");
    writeFileSync(manifest, JSON.stringify({ ...JSON.parse(original) as object, peerDependencies: { "@neralabs/rerail": ">=9.0.0" } }));
    expect((await rerail(["packs", "list"])).stdout).toMatch(/left out: @neralabs\/rerail-pack-javascript@0\.1\.1: .*>=9\.0\.0/u);
    expect((await eqeqeq()).unavailable).toContain("javascript: missing");
    writeFileSync(manifest, original);

    const removed = await rerail(["packs", "remove", "javascript"]);
    expect(removed.code).toBe(0);
    expect(await eqeqeq()).toEqual({ on: undefined, unavailable: ["javascript: missing"] });
    // Installing it again restores what the repository had.
    await rerail(["packs", "add", `javascript@${newer}`]);
    expect(await eqeqeq()).toEqual({ on: false, unavailable: [] });
  });
});
