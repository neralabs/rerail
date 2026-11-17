import type { DaemonMachine } from "../../../src/runtime/index.js";

/**
 * A machine on `platform` where nothing reaches this one unless `overrides` says so: every pid is gone, no named pipe
 * exists, every engine refuses the capability proof, no main-instance or home daemon lock can be tried (as without
 * the native binding), the environment is empty, and a spawn fails the test. A test names the OS calls it checks, so
 * each platform's branch of the daemon control runs on every OS.
 */
export function fakeMachine(platform: NodeJS.Platform, overrides: Partial<DaemonMachine> = {}): DaemonMachine {
  const refuse = (what: string) => (): never => {
    throw new Error(`this test starts no ${what}`);
  };
  return {
    platform,
    kill: () => {
      throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
    },
    env: {},
    pipes: () => [],
    capabilityProof: async () => "refused",
    forgetMainInstanceHolder: () => undefined,
    mainInstanceLockHeld: () => undefined,
    homeDaemonLockHeld: () => undefined,
    nodeExecutable: () => "node",
    spawnDetached: refuse("process through the native spawn"),
    spawn: refuse("process through Node's spawn"),
    spawnShown: refuse("browser"),
    ...overrides,
  };
}
