// The install checks' assertion that a managed install holds this machine's engine package and no other
// (check_engine_package in scripts/host/azure/guest.sh, Assert-RerailEnginePackage in scripts/host/windows-host.ps1).
// Each guest carries the check as a node script of its own, since a VM has no checkout to read one from; they run only
// in a release's install gates and the environment matrix, so the script is run here, on installs laid out by hand.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { linkFolder } from "../../support/platform/links.ts";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

const HOST = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "scripts", "host");
/** The text of the host script `file` between `start` and `end`, whatever line endings the checkout gave it. */
function between(file, start, end) {
  const text = readFileSync(join(HOST, file), "utf8").replaceAll("\r\n", "\n");
  const from = text.indexOf(start);
  const to = text.indexOf(end, from);
  if (from === -1 || to === -1) throw new Error(`${file} no longer holds the engine package check between ${JSON.stringify(start)} and ${JSON.stringify(end)}`);
  return text.slice(from + start.length, to);
}
/** guest.sh hands the script to `node -e`, indented; windows-host.ps1 writes it to a file, whose path is one more argument. */
const BASH_CHECK = between(join("azure", "guest.sh"), "check_engine_package() {\n  node -e '\n", "\n  ' \"${RERAIL_HOME");
const WINDOWS_CHECK = between("windows-host.ps1", "Write-RerailUtf8 $check @'\n", "\n'@");

const folders = [];
afterEach(() => {
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});

const OWN = { os: [process.platform], cpu: [process.arch] };
const OTHER = { os: ["plan9"], cpu: [process.arch] };

/**
 * A Rerail home whose `current` links to core in a managed install: core pins `pinned` (engine package names), and
 * `beside` and `inside` (name to its `os` and `cpu`) are the engine packages installed beside core and inside it.
 */
function managedInstall({ pinned = ["@scope/engine-own", "@scope/engine-other"], beside = {}, inside = {} }) {
  const home = mkdtempSync(join(tmpdir(), "rerail-engine-package-check-"));
  folders.push(home);
  const prefix = join(home, "versions", "1.2.0");
  const core = join(prefix, "node_modules", "@neralabs", "rerail");
  const install = (folder, manifest) => {
    mkdirSync(folder, { recursive: true });
    writeFileSync(join(folder, "package.json"), JSON.stringify(manifest));
  };
  install(core, { name: "@neralabs/rerail", version: "1.2.0", ...(pinned.length === 0 ? {} : { optionalDependencies: Object.fromEntries(pinned.map((name) => [name, "1.2.0"])) }) });
  for (const [name, platform] of Object.entries(beside)) install(join(prefix, "node_modules", name), { name, version: "1.2.0", ...platform });
  for (const [name, platform] of Object.entries(inside)) install(join(core, "node_modules", name), { name, version: "1.2.0", ...platform });
  linkFolder(core, join(home, "current"));
  return home;
}

/** Runs the Windows copy of the check, which reads its arguments as the file it is written to does. */
function check(home, packed = true) {
  const script = join(home, "check.js");
  writeFileSync(script, WINDOWS_CHECK);
  return spawnSync(process.execPath, [script, join(home, "current"), ...(packed ? ["packed"] : [])], { encoding: "utf8" });
}

describe("the install checks' engine package assertion", () => {
  it("is one script on Linux, macOS and Windows", () => {
    const unindented = BASH_CHECK.split("\n").map((line) => line.replace(/^ {4}/u, "")).join("\n");
    expect(unindented.replace("process.argv.slice(1)", "process.argv.slice(2)")).toBe(WINDOWS_CHECK);
    expect(WINDOWS_CHECK).toContain("process.argv.slice(2)");
  });

  it("passes an install holding this machine's engine package alone, beside core, and names it", () => {
    const run = check(managedInstall({ beside: { "@scope/engine-own": OWN } }));
    expect(run.status).toBe(0);
    expect(run.stdout).toContain(join("node_modules", "@scope", "engine-own"));
  });

  it("fails an install holding every engine package core pins, as npm lays them out inside core by itself", () => {
    const run = check(managedInstall({ inside: { "@scope/engine-own": OWN, "@scope/engine-other": OTHER } }));
    expect(run.status).toBe(1);
    expect(run.stdout).toContain(`expected one of the 2 core 1.2.0 pins: the one for ${process.platform} ${process.arch}`);
  });

  it("fails an install with no engine package, and one holding only another machine's", () => {
    const none = check(managedInstall({}));
    expect(none.status).toBe(1);
    expect(none.stdout).toContain("engine packages in");
    expect(none.stdout).toContain(": none\n");
    expect(check(managedInstall({ beside: { "@scope/engine-other": OTHER } })).status).toBe(1);
  });

  it("fails a packed core that pins no engine package, and passes one from npmjs, which may be from before they existed", () => {
    const home = managedInstall({ pinned: [] });
    expect(check(home).status).toBe(1);
    const published = check(home, false);
    expect(published.status).toBe(0);
    expect(published.stdout).toContain("core 1.2.0 pins no engine package: a version from before they existed");
  });
});
