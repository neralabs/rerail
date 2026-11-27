import { afterEach, beforeEach, vi } from "vitest";

/**
 * Turns autostart on (`RERAIL_AUTOSTART=1`) for every test of the file (or `describe`) that calls it, as a person's
 * shell has it, and gives the variable back the value it had before after every test. The suite keeps it `0`
 * (`vitest.config.ts`), so no other test starts a daemon; OMP's and Pi's attach starts or replaces one only with it on.
 * A process a test spawns inherits the stub.
 */
export function withAutostartOn(): void {
  let before: string | undefined;
  beforeEach(() => {
    before = process.env.RERAIL_AUTOSTART;
    vi.stubEnv("RERAIL_AUTOSTART", "1");
  });
  afterEach(() => {
    vi.stubEnv("RERAIL_AUTOSTART", before);
  });
}
