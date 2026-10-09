import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, readlinkSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "vitest";

import {
  backupRootFor,
  configBackupName,
  main as nuke,
  parseNukeArguments,
  parseStatusRegistration,
  planNuke,
  rerailHomeMatcher,
  stripClaudeSettings,
  stripHookCommands,
  stripOmpMcp,
  stripPiSettings,
} from "./rerail-nuke.mjs";
import { loadSqlite, timestamp } from "./rerail-restore.mjs";

const NUKE_SCRIPT = fileURLToPath(new URL("./rerail-nuke.mjs", import.meta.url));
const { DatabaseSync } = await loadSqlite();

async function withHome(run) {
  const home = mkdtempSync(join(tmpdir(), "rerail-nuke-test-"));
  try {
    return await run(home);
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
}

/** A sandbox environment: HOME is the temp home and the dev-checkout registry lives under it too. */
function envFor(home) {
  return { HOME: home, PATH: process.env.PATH ?? "", XDG_STATE_HOME: join(home, ".local", "state") };
}

/** A scripts/rerail-dev.mjs double: `checkouts` maps a checkout to its instance; stops are recorded. */
function fakeDev(checkouts = {}) {
  const stopped = [];
  return {
    stopped,
    listDevCheckouts: () => Object.keys(checkouts),
    readDevInstance: checkout => checkouts[checkout],
    stopDevCheckout: async checkout => {
      stopped.push(checkout);
      return { checkout, home: checkouts[checkout]?.home, stopped: true };
    },
  };
}

function writeJson(path, value) {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** Every file and symlink under `root` with its contents or target, for exact before/after comparison. */
function snapshot(root) {
  const entries = {};
  const walk = directory => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) entries[path] = `-> ${readlinkSync(path)}`;
      else if (entry.isDirectory()) walk(path);
      else entries[path] = `${statSync(path).mode}:${readFileSync(path, "base64")}`;
    }
  };
  walk(root);
  return entries;
}

function statusDatabase(rerailHome, rows) {
  mkdirSync(rerailHome, { recursive: true });
  const database = new DatabaseSync(join(rerailHome, "harness.sqlite"));
  database.exec(`CREATE TABLE harness_status_lines (settings_path text PRIMARY KEY NOT NULL, installed_command text NOT NULL,
    installed_root text NOT NULL, previous_json text NOT NULL)`);
  const insert = database.prepare("INSERT INTO harness_status_lines VALUES (?, ?, ?, ?)");
  for (const [path, previous] of rows) insert.run(path, "node claude-status.js", join(rerailHome, "current"), JSON.stringify(previous));
  database.close();
}

function ruleDatabase(directory, rows) {
  mkdirSync(directory, { recursive: true });
  const database = new DatabaseSync(join(directory, "rerail.sqlite"));
  database.exec("CREATE TABLE library_rules (id text PRIMARY KEY)");
  const insert = database.prepare("INSERT INTO library_rules VALUES (?)");
  for (const id of rows) insert.run(id);
  database.close();
}

function ruleIds(path) {
  const database = new DatabaseSync(path, { readOnly: true });
  try {
    return database.prepare("SELECT id FROM library_rules ORDER BY id").all().map(row => row.id);
  } finally {
    database.close();
  }
}

/** A machine with Rerail registered in every coding agent, plus unrelated settings that must survive. */
function installFixture(home) {
  const rerailHome = join(home, ".rerail");
  const cli = `${rerailHome}/current/dist/cli.js`;
  const hook = name => ({ type: "command", command: `node "${rerailHome}/current/dist/${name}"` });
  const userHook = { type: "command", command: "echo user" };
  mkdirSync(join(rerailHome, "current", "dist"), { recursive: true });
  writeFileSync(join(rerailHome, "update-state.json"), "{}\n");
  ruleDatabase(rerailHome, ["rule-a", "rule-b"]);
  const settings = join(home, ".claude", "settings.json");
  const projectSettings = join(home, "work", "repo", ".claude", "settings.local.json");
  statusDatabase(rerailHome, [
    [settings, { type: "command", command: "printf user-status", padding: 3 }],
    [projectSettings, { previousInherited: true, previous: { type: "command", command: "printf shared" } }],
  ]);
  writeJson(settings, {
    model: "opus",
    hooks: {
      SessionStart: [{ hooks: [hook("claude-hook.js")] }, { hooks: [userHook] }],
      PreToolUse: [{ matcher: "", hooks: [hook("claude-hook.js"), userHook] }],
      Stop: [{ hooks: [hook("claude-hook.js")] }],
      Empty: [],
    },
    statusLine: { type: "command", command: `node ${rerailHome}/current/dist/claude-status.js`, padding: 3 },
    enabledPlugins: { "rerail@rerail-dev": true, "other@market": true },
    extraKnownMarketplaces: { "rerail-dev": { source: { source: "directory", path: "/x" } } },
  });
  writeJson(projectSettings, { statusLine: { type: "command", command: `node ${rerailHome}/current/dist/claude-status.js ${projectSettings}` } });
  writeJson(join(home, ".cursor", "hooks.json"), {
    version: 1,
    hooks: { beforeSubmitPrompt: [{ command: `node ${rerailHome}/current/dist/cursor-hook.js` }, { command: "echo cursor-user" }], stop: [hook("cursor-hook.js")] },
  });
  writeJson(join(home, ".codex", "hooks.json"), {
    hooks: { SessionStart: [{ hooks: [hook("codex-hook.js")] }], PostToolUse: [{ matcher: "", hooks: [userHook, hook("codex-hook.js")] }] },
  });
  writeJson(join(home, ".omp", "agent", "mcp.json"), {
    mcpServers: { rerail: { command: "node", args: [cli, "mcp-bridge"] }, other: { command: "other" } },
  });
  mkdirSync(join(home, ".omp", "agent", "extensions"), { recursive: true });
  symlinkSync(join(rerailHome, "current"), join(home, ".omp", "agent", "extensions", "rerail"));
  writeJson(join(home, ".pi", "agent", "settings.json"), { packages: [join(rerailHome, "current"), "npm:other"], theme: "dark" });
  mkdirSync(join(home, ".claude", "commands"), { recursive: true });
  for (const name of ["rerail.md", "rerail-ui.md"]) writeFileSync(join(home, ".claude", "commands", name), `Run ${cli} ${name}\n`);
  for (const name of ["rerail-rules.md", "rerail.md.bak"]) writeFileSync(join(home, ".claude", "commands", name), name);
  writeFileSync(join(home, ".claude", "commands", "rerail-session.md"), "someone else's command\n");
  mkdirSync(join(home, ".cursor", "commands"), { recursive: true });
  writeFileSync(join(home, ".cursor", "commands", "rerail-start.md"), `node ${cli} start\n`);
  mkdirSync(join(home, ".codex", "skills", "rerail"), { recursive: true });
  writeFileSync(join(home, ".codex", "skills", "rerail", "SKILL.md"), `node ${cli}\n<!-- Managed by @neralabs/rerail -->\n`);
  return { rerailHome, settings, projectSettings };
}

test("argument parsing accepts --dry-run, --yes and --help only", () => {
  assert.deepEqual(parseNukeArguments([]), { dryRun: false, yes: false, help: false });
  assert.deepEqual(parseNukeArguments(["--dry-run", "--yes"]), { dryRun: true, yes: true, help: false });
  assert.throws(() => parseNukeArguments(["--force"]), /usage: just nuke-rerail/u);
});

test("the backup root is dated, and timed when today's already exists", () => {
  const now = new Date(2026, 8, 29, 7, 5, 9);
  assert.equal(backupRootFor("/h", now, () => false), "/h/.rerail-clean-20260929");
  assert.equal(backupRootFor("/h", now, path => path === "/h/.rerail-clean-20260929"), "/h/.rerail-clean-20260929-070509");
  assert.equal(timestamp(now), "20260929-070509");
});

test("config backups are named by their path under HOME with separators as underscores", () => {
  assert.equal(configBackupName("/h/.claude/settings.json", "/h"), ".claude_settings.json");
  assert.equal(configBackupName("/h/.omp/agent/mcp.json", "/h"), ".omp_agent_mcp.json");
  assert.equal(configBackupName("/elsewhere/repo/.claude/settings.local.json", "/h"), "elsewhere_repo_.claude_settings.local.json");
});

test("the matcher recognises the Rerail home by path, realpath, and ~ or $HOME spelling only", () => {
  const matches = rerailHomeMatcher("/h/.rerail", "/h");
  assert.ok(matches("node /h/.rerail/current/dist/claude-hook.js"));
  assert.ok(matches("node ~/.rerail/current/dist/x.js"));
  assert.ok(matches('node "$HOME/.rerail/current/dist/x.js"'));
  assert.ok(matches("/h/.rerail"));
  assert.ok(!matches("node /h/.rerail-other/x.js"));
  assert.ok(!matches("node /h/project/.rerail/x.js"));
});

test("hook stripping handles grouped and flat entries and drops only events it emptied", () => {
  const matches = rerailHomeMatcher("/h/.rerail", "/h");
  const rerail = { command: "node /h/.rerail/current/dist/hook.js" };
  const execForm = { command: "node", args: ["/h/.rerail/current/dist/hook.js"] };
  const user = { command: "echo user" };
  const result = stripHookCommands({
    A: [{ matcher: "", hooks: [rerail, user] }],
    B: [{ hooks: [rerail, execForm] }],
    C: [rerail, user],
    D: [],
    E: "malformed",
  }, matches);
  assert.equal(result.removed, 4);
  assert.deepEqual(result.hooks, { A: [{ matcher: "", hooks: [user] }], C: [user], D: [], E: "malformed" });
});

test("Claude settings restore the recorded status line, or drop it when none or inherited", () => {
  const matches = rerailHomeMatcher("/h/.rerail", "/h");
  const rerailStatus = { statusLine: { type: "command", command: "node /h/.rerail/current/dist/claude-status.js" } };
  const previous = { type: "command", command: "printf mine", padding: 2 };
  assert.deepEqual(stripClaudeSettings(rerailStatus, { matches, registration: { previous } }).value, { statusLine: previous });
  assert.deepEqual(stripClaudeSettings(rerailStatus, { matches, registration: undefined }).value, {});
  assert.deepEqual(stripClaudeSettings(rerailStatus, { matches, registration: { previous: null } }).value, {});
  assert.deepEqual(stripClaudeSettings(rerailStatus, { matches, registration: { previous, inherited: true } }).value, {});
  const own = { statusLine: { type: "command", command: "printf mine" } };
  assert.equal(stripClaudeSettings(own, { matches, registration: { previous } }).value, own);
  assert.deepEqual(parseStatusRegistration(JSON.stringify({ previousInherited: true, previous })), { previous, inherited: true });
  assert.deepEqual(parseStatusRegistration("null"), { previous: null });
});

test("Claude settings lose the rerail-dev plugin and marketplace but keep other plugins", () => {
  const matches = rerailHomeMatcher("/h/.rerail", "/h");
  const { value, removed } = stripClaudeSettings({
    enabledPlugins: { "rerail@rerail-dev": true, "x@y": false },
    extraKnownMarketplaces: { "rerail-dev": {} },
  }, { matches });
  assert.deepEqual(value, { enabledPlugins: { "x@y": false } });
  assert.equal(removed.length, 2);
});

test("OMP keeps a rerail MCP server that does not point into the Rerail home; Pi drops only Rerail packages", () => {
  const matches = rerailHomeMatcher("/h/.rerail", "/h");
  const foreign = { mcpServers: { rerail: { command: "node", args: ["/elsewhere/cli.js"] } } };
  assert.equal(stripOmpMcp(foreign, { matches }).value, foreign);
  assert.deepEqual(stripOmpMcp({ mcpServers: { rerail: { command: "node", args: ["/h/.rerail/current/dist/cli.js"] } } }, { matches }).value, { mcpServers: {} });
  const pi = stripPiSettings({ packages: ["../../.rerail/current", "npm:x"] }, { matches, agentDir: "/h/.pi/agent" });
  assert.deepEqual(pi.value, { packages: ["npm:x"] });
  assert.deepEqual(stripPiSettings({ packages: ["/h/.rerail/current"] }, { matches, agentDir: "/h/.pi/agent" }).value, {});
});

test("nuke --yes without a working launcher removes every Rerail trace itself, backs it up restorably, and keeps the rest", async () => {
  await withHome(async home => {
    const { rerailHome, settings, projectSettings } = installFixture(home);
    const before = readFileSync(settings, "utf8");
    const lines = [];
    assert.equal(await nuke(["--yes"], { env: envFor(home), log: line => lines.push(line), interactive: false, dev: fakeDev() }), 0);
    const backup = backupRootFor(home, new Date(), () => false);

    assert.equal(readFileSync(join(backup, "configs", ".claude_settings.json"), "utf8"), before);
    for (const name of [".cursor_hooks.json", ".codex_hooks.json", ".omp_agent_mcp.json", ".pi_agent_settings.json", "work_repo_.claude_settings.local.json"]) {
      assert.ok(existsSync(join(backup, "configs", name)), name);
    }
    assert.deepEqual(readJson(settings), {
      model: "opus",
      hooks: {
        SessionStart: [{ hooks: [{ type: "command", command: "echo user" }] }],
        PreToolUse: [{ matcher: "", hooks: [{ type: "command", command: "echo user" }] }],
        Empty: [],
      },
      statusLine: { type: "command", command: "printf user-status", padding: 3 },
      enabledPlugins: { "other@market": true },
    });
    assert.ok(readFileSync(settings, "utf8").endsWith("}\n"));
    assert.deepEqual(readJson(projectSettings), {});
    assert.deepEqual(readJson(join(home, ".cursor", "hooks.json")), { version: 1, hooks: { beforeSubmitPrompt: [{ command: "echo cursor-user" }] } });
    assert.deepEqual(readJson(join(home, ".codex", "hooks.json")), { hooks: { PostToolUse: [{ matcher: "", hooks: [{ type: "command", command: "echo user" }] }] } });
    assert.deepEqual(readJson(join(home, ".omp", "agent", "mcp.json")), { mcpServers: { other: { command: "other" } } });
    assert.deepEqual(readJson(join(home, ".pi", "agent", "settings.json")), { packages: ["npm:other"], theme: "dark" });
    assert.ok(!existsSync(join(home, ".omp", "agent", "extensions", "rerail")));

    assert.deepEqual(readdirSync(join(backup, "claude-commands")).sort(), ["rerail-ui.md", "rerail.md"]);
    assert.deepEqual(readdirSync(join(home, ".claude", "commands")).sort(), ["rerail-rules.md", "rerail-session.md", "rerail.md.bak"], "commands that are not Rerail's stay");
    assert.deepEqual(readdirSync(join(backup, "cursor-commands")), ["rerail-start.md"]);
    assert.match(readFileSync(join(backup, "codex-skill-rerail", "SKILL.md"), "utf8"), /Managed by @neralabs\/rerail/u);
    assert.ok(!existsSync(join(home, ".codex", "skills", "rerail")));

    assert.ok(!existsSync(rerailHome));
    assert.deepEqual(ruleIds(join(backup, "rerail-home", "rerail.sqlite")), ["rule-a", "rule-b"]);
    assert.ok(existsSync(join(backup, "rerail-home", "harness.sqlite")));
    assert.ok(lines.some(line => line.includes("Restart open Claude/Cursor/Codex/OMP/Pi sessions")));

    assert.equal(await nuke(["--yes"], { env: envFor(home), log: () => {}, interactive: false, dev: fakeDev() }), 0, "a second run is a no-op");
    assert.equal(readdirSync(home).filter(name => name.startsWith(".rerail-clean-")).length, 1, "nothing left means no second backup");
  });
});

test("with a launcher, configs are backed up and commands moved before rerail install --remove runs, then the rest is swept", async () => {
  await withHome(async home => {
    const { rerailHome, settings } = installFixture(home);
    const calls = join(home, "launcher-calls");
    const backup = backupRootFor(home, new Date(), () => false);
    mkdirSync(join(rerailHome, "bin"), { recursive: true });
    // The launcher records what it saw: whether the backup already held the settings and the command, and its arguments.
    writeFileSync(join(rerailHome, "bin", "rerail"), [
      "#!/bin/sh",
      `[ -f '${join(backup, "configs", ".claude_settings.json")}' ] && b=backed-up || b=not-backed-up`,
      `[ -f '${join(home, ".claude", "commands", "rerail.md")}' ] && c=command-present || c=command-moved`,
      `printf '%s %s %s %s\\n' "$RERAIL_HOME" "$b" "$c" "$*" >> '${calls}'`,
      "",
    ].join("\n"), { mode: 0o755 });
    assert.equal(await nuke(["--yes"], { env: envFor(home), log: () => {}, interactive: false, dev: fakeDev() }), 0);
    assert.deepEqual(readFileSync(calls, "utf8").trim().split("\n"), [
      `${rerailHome} not-backed-up command-present daemon stop`,
      `${rerailHome} backed-up command-moved install --remove --harness omp,pi,claude-code,codex,cursor`,
    ]);
    assert.equal(readJson(settings).statusLine.command, "printf user-status", "the sweep finishes what the launcher left");
    assert.ok(existsSync(join(backup, "rerail-home", "bin", "rerail")));
  });
});

test("dry run prints the plan and changes nothing", async () => {
  await withHome(async home => {
    installFixture(home);
    const before = snapshot(home);
    const lines = [];
    assert.equal(await nuke(["--dry-run"], { env: envFor(home), log: line => lines.push(line), interactive: false, dev: fakeDev() }), 0);
    assert.deepEqual(snapshot(home), before);
    assert.ok(lines.some(line => line.includes("Dry run")));
    assert.ok(lines.some(line => line.includes(".claude/settings.json")));
  });
});

test("without --yes a non-interactive run refuses and changes nothing", async () => {
  await withHome(async home => {
    installFixture(home);
    const before = snapshot(home);
    const result = spawnSync(process.execPath, [NUKE_SCRIPT], { env: envFor(home), encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /--yes/u);
    assert.deepEqual(snapshot(home), before);
  });
});

test("a machine without Rerail has nothing to remove and gets no backup", async () => {
  await withHome(async home => {
    writeJson(join(home, ".claude", "settings.json"), { model: "opus" });
    const plan = await planNuke({ env: envFor(home), dev: fakeDev() });
    assert.deepEqual(plan.steps, []);
    const lines = [];
    assert.equal(await nuke([], { env: envFor(home), log: line => lines.push(line), interactive: false, dev: fakeDev() }), 0);
    assert.match(lines.join("\n"), /Nothing to remove/u);
    assert.deepEqual(readdirSync(home), [".claude"]);
  });
});

test("RERAIL_HOME relocates the home the nuke removes and the hooks it matches", async () => {
  await withHome(async home => {
    const rerailHome = join(home, "custom-rerail");
    mkdirSync(join(rerailHome, "current"), { recursive: true });
    writeJson(join(home, ".codex", "hooks.json"), { hooks: { Stop: [{ hooks: [{ command: `node ${rerailHome}/current/dist/codex-hook.js` }] }] } });
    assert.equal(await nuke(["--yes"], { env: { ...envFor(home), RERAIL_HOME: rerailHome }, log: () => {}, interactive: false, dev: fakeDev() }), 0);
    assert.ok(!existsSync(rerailHome));
    assert.deepEqual(readJson(join(home, ".codex", "hooks.json")), {});
  });
});

test("only running checkout dev daemons on their own dev home get a stop step; a dry run stops none", async () => {
  await withHome(async home => {
    const sleeper = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
    try {
      const running = join(home, "running");
      const idle = join(home, "idle");
      const bridged = join(home, "bridged");
      for (const [checkout, pid] of [[running, sleeper.pid], [idle, 999_999_999]]) {
        writeJson(join(checkout, ".rerail", "dev", "home", "daemon.json"), { pid });
      }
      writeJson(join(home, ".rerail", "daemon.json"), { pid: sleeper.pid });
      const instance = checkout => ({ checkout, home: join(checkout, ".rerail", "dev", "home"), port: 1, url: "http://127.0.0.1:1" });
      const dev = fakeDev({ [running]: instance(running), [idle]: instance(idle), [bridged]: { ...instance(bridged), home: join(home, ".rerail") } });

      const lines = [];
      assert.equal(await nuke(["--dry-run"], { env: envFor(home), log: line => lines.push(line), interactive: false, dev }), 0);
      assert.ok(lines.some(line => line.includes(`Stop the dev daemon of ${running} (pid ${sleeper.pid}`)));
      assert.ok(!lines.some(line => line.includes(idle) || line.includes(bridged)));
      assert.deepEqual(dev.stopped, []);

      const plan = await planNuke({ env: envFor(home), dev });
      assert.deepEqual(plan.steps.map(step => step.kind).slice(0, 2), ["daemon", "dev-daemon"]);
      await plan.steps[1].run(() => {});
      assert.deepEqual(dev.stopped, [running]);
    } finally {
      sleeper.kill("SIGKILL");
    }
  });
});
