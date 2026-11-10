import { ADAPTER_CAPABILITIES, steerBlock, type RerailHost, type SteerRule, type SteerTrigger } from "../../src/engine/index.js";

/** An edit's steer for one finding of `ruleId`, as the code-rule enforcer renders it. */
export function editSteer(ruleId: string, message: string, location?: string): string {
  return steerBlock({ kind: "edit" }, [{ ruleId, mode: "steer", message, location }]);
}

/** The rules a delivery names for `ruleIds`, each steered, and linked to its page on `dashboard` when one is given. */
export function steeredRules(ruleIds: readonly string[], dashboard?: string): SteerRule[] {
  return ruleIds.map(id => ({
    id,
    state: "steered",
    ...(dashboard === undefined ? {} : { url: `${dashboard}#rule/${encodeURIComponent(id)}` }),
  }));
}

type SteerShell = NonNullable<Parameters<typeof steerBlock>[2]>;

/**
 * The shell `host`'s agent runs the commands a steer names in, on the machine the test runs on: sh off Windows; on
 * Windows the shell its capability profile names (`windowsCommandShell`), PowerShell or else Git Bash.
 */
export function hostShell(host: RerailHost): SteerShell {
  if (process.platform !== "win32") return "posix";
  return ADAPTER_CAPABILITIES[host].windowsCommandShell === "powershell" ? "powershell" : "git-bash";
}

/**
 * The line that ends a steer about `ruleId` that the runtime sends `host`'s agent on this machine, whose feedback
 * command is written for that agent's shell (`hostShell`).
 */
export function steerLatitude(host: RerailHost, ruleId: string, trigger: SteerTrigger = { kind: "edit" }): string {
  return steerBlock(trigger, [{ ruleId, mode: "steer", message: "-" }], hostShell(host)).split("\n").at(-1)!;
}
