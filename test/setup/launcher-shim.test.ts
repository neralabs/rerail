import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";
import { gitBashShim, launcherShim } from "../../src/setup/updater.js";

// The launcher texts alone: nothing here installs a version or writes a launcher.
describe("the Git Bash launcher", () => {
  const shim = gitBashShim(String.raw`C:\Users\me\.rerail\current`, String.raw`C:\Program Files\nodejs\node.exe`);
  const cli = String.raw`'C:\Users\me\.rerail\current\dist\cli\cli.js'`;

  it("starts node through winpty in Git Bash's mintty window, where node would get pipes instead of a console", () => {
    expect(shim).toContain('if [ -t 0 ] && [ -t 1 ] && [ "${TERM_PROGRAM-}" = mintty ] && [ -x /usr/bin/winpty ]; then');
    expect(shim).toContain(`*) exec /usr/bin/winpty "$NODE" ${cli} "$@" ;;`);
    // A pseudo console already gives node a console; pipes and every other terminal start node as before.
    expect(shim).toContain("*enable_pcon*) ;;");
    expect(shim.trimEnd().split("\n").at(-1)).toBe(`exec "$NODE" ${cli} "$@"`);
  });

  it("is a script sh reads, and the macOS and Linux launcher never takes winpty", () => {
    const checked = spawnSync("sh", ["-n"], { input: shim, encoding: "utf8" });
    if (checked.error === undefined) expect(checked.status, checked.stderr).toBe(0);
    expect(launcherShim("/home/me/.rerail/current", "/usr/bin/node", "linux")).not.toContain("winpty");
  });
});
