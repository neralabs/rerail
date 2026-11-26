import { type ChildProcessByStdio, execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { join } from "node:path";
import { createInterface } from "node:readline";
import type { Readable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { backupPrivilegeSwitch, PRIVILEGE_SWITCH_START_BOUND_MS } from "./backup-privileges.js";
import { PrivilegeSwitch, powerShellWords, startPrivilegeSwitch, WINDOWS_POWERSHELL } from "./privileges/privilege-switch.js";

/** A case's time limit: room for two answers of a PowerShell that runs, which take milliseconds, never for a start. */
const CASE_LIMIT_MS = 2_000;
/** How long the stand-in holds Windows PowerShell's start back: twice {@link CASE_LIMIT_MS}. */
const HELD_START_MS = 4_000;
const BACKUP_PRIVILEGES = ["SeBackupPrivilege", "SeRestorePrivilege"];

/** Windows PowerShell, started only once `held-start.mjs` has waited {@link HELD_START_MS}, as on a new machine. */
const heldPowerShell = [process.execPath, join(import.meta.dirname, "privileges", "held-start.mjs"), String(HELD_START_MS), ...WINDOWS_POWERSHELL] as const;

/** Each privilege this process's token holds and its state (`Enabled`, `Disabled`), as Windows' own whoami lists them. */
function privilegeStates(): Map<string, string> {
  // A process started now takes a copy of this process's token, its privileges' states included.
  const listed = execFileSync(join(process.env.SystemRoot ?? "C:\\Windows", "System32", "whoami.exe"), ["/priv", "/fo", "csv", "/nh"], { encoding: "utf8" });
  return new Map([...listed.matchAll(/^"([^"]+)","[^"]*","([^"]+)"\s*$/gmu)].map(([, name, state]) => [name ?? "", state ?? ""]));
}

/** Whether a process `pid` runs. */
function running(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

describe.runIf(process.platform === "win32")("cases that turn this process's Backup and Restore privileges off", () => {
  const { withoutBackupPrivileges } = backupPrivilegeSwitch(heldPowerShell);

  it("run within a time limit shorter than their PowerShell's start, which the set-up waited for", async () => {
    expect(await withoutBackupPrivileges(async () => "ran")).toBe("ran");
  }, CASE_LIMIT_MS);

  it("pass a body's error on to the case, and their PowerShell answers the next body", async () => {
    await expect(withoutBackupPrivileges(() => Promise.reject(new Error("the body failed")))).rejects.toThrow("the body failed");
    expect(await withoutBackupPrivileges(async () => "ran")).toBe("ran");
  }, CASE_LIMIT_MS);

  it("run the body with both off and turn on again those that were on, after a body that throws too", async ({ skip }) => {
    const before = privilegeStates();
    if (!BACKUP_PRIVILEGES.every(name => before.get(name) === "Enabled")) skip("this process's token holds Backup and Restore off");
    const inside = await withoutBackupPrivileges(async () => privilegeStates());
    expect(BACKUP_PRIVILEGES.map(name => inside.get(name))).toEqual(["Disabled", "Disabled"]);
    expect(privilegeStates()).toEqual(before);
    await expect(withoutBackupPrivileges(() => Promise.reject(new Error("the body failed")))).rejects.toThrow("the body failed");
    expect(privilegeStates()).toEqual(before);
  });
});

describe.runIf(process.platform === "win32")("a privilege switch's PowerShell", () => {
  describe("when the switch is ended", () => {
    let kept: PrivilegeSwitch;
    beforeAll(async () => {
      kept = await startPrivilegeSwitch();
    }, PRIVILEGE_SWITCH_START_BOUND_MS);

    it("has exited once the end resolves", async () => {
      const pid = kept.pid ?? 0;
      expect(running(pid)).toBe(true);
      await kept.end();
      expect(running(pid)).toBe(false);
    });
  });

  describe("when the test process that started it is ended from outside", () => {
    let holder: ChildProcessByStdio<null, Readable, null>;
    let powershell = 0;
    beforeAll(async () => {
      holder = spawn(process.execPath, [join(import.meta.dirname, "privileges", "switch-holder.mjs"), join(import.meta.dirname, "privileges", "privilege-switch.ts")], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
      const printed = once(createInterface({ input: holder.stdout }), "line") as Promise<[string]>;
      const ended = once(holder, "exit").then(([code]: unknown[]) => Promise.reject(new Error(`switch-holder.mjs ended (exit code ${String(code)}) before its switch was ready`)));
      powershell = Number((await Promise.race([printed, ended]))[0]);
    }, PRIVILEGE_SWITCH_START_BOUND_MS);
    afterAll(() => {
      holder.kill();
    });

    it("ends with it, on the end of its input", async () => {
      expect(running(powershell)).toBe(true);
      holder.kill();
      await once(holder, "exit");
      // Windows offers Node no wait on another process's end: look until it has gone, within this case's limit.
      while (running(powershell)) await delay(50);
    });
  });

  describe("when it ends before it is ready", () => {
    let failure: unknown;
    beforeAll(async () => {
      // PowerShell takes every argument after -Command as the command's text: the switch's own arguments follow the #.
      const failing = startPrivilegeSwitch([...WINDOWS_POWERSHELL, "-NoProfile", "-Command", "throw 'no start here'; #"]);
      failure = await failing.then(kept => kept.end(), (error: unknown) => error);
    }, PRIVILEGE_SWITCH_START_BOUND_MS);

    it("ends the switch's start in PowerShell's own words", () => {
      expect(failure).toBeInstanceOf(Error);
      expect((failure as Error).message).toMatch(/ended \(exit code 1\): no start here/u);
    });
  });
});

describe("a privilege switch whose program ends before it is ready", () => {
  it("ends its start in that program's own words", async () => {
    await expect(startPrivilegeSwitch([process.execPath, "--no-such-option"])).rejects.toThrow(/ended \(exit code 9\): .*bad option: --no-such-option/u);
  });

  it("ends it in the words of PowerShell's error records, which -EncodedCommand writes as CLIXML", () => {
    const errorOutput = [
      "#< CLIXML",
      '<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04"><Obj S="progress" RefId="0"><TN RefId="0"><T>System.Management.Automation.PSCustomObject</T><T>System.Object</T></TN><MS><I64 N="SourceId">1</I64><PR N="Record"><AV>Preparing modules for first use.</AV><AI>0</AI><Nil /><PI>-1</PI><PC>-1</PC><T>Completed</T><SR>-1</SR><SD> </SD></PR></MS></Obj>' +
        '<S S="Error">OpenProcessToken failed: 5_x000D__x000A_</S><S S="Error">At line:9 char:79 &amp; &lt;here&gt;_x000D__x000A_</S></Objs>',
    ].join("\r\n");
    expect(powerShellWords(errorOutput)).toBe("OpenProcessToken failed: 5\r\nAt line:9 char:79 & <here>");
  });
});

describe("a privilege switch ended while its program still starts", () => {
  it("ends that program at once, as a set-up cut at its bound leaves it to the tear-down", async () => {
    // A start held ten minutes: only ending the program, not closing its input, ends the switch within this case.
    const starting = new PrivilegeSwitch([process.execPath, join(import.meta.dirname, "privileges", "held-start.mjs"), "600000", process.execPath]);
    const ready = starting.ready().then(() => undefined, (error: unknown) => error);
    await starting.end();
    expect(running(starting.pid ?? 0)).toBe(false);
    expect(await ready).toBeInstanceOf(Error);
  });
});
