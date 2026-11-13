// Cursor's transcript state is replaced through a temporary file: a rename that fails leaves none behind.
// node:fs/promises is spied on so the test can make the rename fail.
import { mkdtemp, readdir, rename } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AdapterSession, RerailEventLoop } from "../../../src/engine/index.js";
import { ingestCursorTranscript } from "../../../src/hosts/cursor/cursor-transcript.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

vi.mock("node:fs/promises", { spy: true });

const roots: string[] = [];
afterEach(async () => {
  vi.mocked(rename).mockReset();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

describe("Cursor's transcript state", () => {
  it("leaves no temporary file behind a rename that failed", async () => {
    const storageRoot = await mkdtemp(join(tmpdir(), "rerail-cursor-state-"));
    roots.push(storageRoot);
    vi.mocked(rename).mockRejectedValueOnce(Object.assign(new Error("EXDEV: cross-device link not permitted"), { code: "EXDEV" }));
    const session = { host: "cursor", sessionId: "session-1", cwd: storageRoot } satisfies AdapterSession;
    const runtime = {} as RerailEventLoop; // Never reached: a prompt submission only records Cursor's text mode.
    await expect(ingestCursorTranscript({ hook_event_name: "beforeSubmitPrompt" }, session, runtime, storageRoot)).rejects.toThrow(/EXDEV/u);
    const temporary = (await readdir(storageRoot, { recursive: true })).filter(name => name.endsWith(".tmp"));
    expect(temporary).toEqual([]);
  });
});
