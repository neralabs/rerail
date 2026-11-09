// The call sites that replace a file or remove a scratch folder Windows may hold: each keeps its own result, or its own
// error, when the rename or the removal fails. node:fs/promises is spied on so a test can make one call fail.
import { mkdir, mkdtemp, readdir, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AdapterSession, RerailEventLoop } from "../../src/engine/adapter-contract.js";
import { ingestCursorTranscript } from "../../src/hosts/cursor/cursor-transcript.js";
import { outsideAnyProject } from "../../src/runtime/npm.js";
import { validateManagedInstallation, type Installation } from "../../src/setup/updater.js";

vi.mock("node:fs/promises", { spy: true });

const roots: string[] = [];
afterEach(async () => {
  vi.mocked(rm).mockReset();
  vi.mocked(rename).mockReset();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

async function folder(prefix: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix));
  roots.push(root);
  return root;
}

const held = (path: unknown): Error => Object.assign(new Error(`EBUSY: resource busy or locked, rmdir '${String(path)}'`), { code: "EBUSY" });

/** The next removal fails as Windows fails one while a process holds a file in the folder; the folder is removed later. */
function failNextRemoval(): void {
  vi.mocked(rm).mockImplementationOnce(async (path) => {
    roots.push(String(path));
    throw held(path);
  });
}

describe("work in a scratch folder Windows may hold", () => {
  it("runs npm outside any project and answers with its result when the folder cannot be removed", async () => {
    failNextRemoval();
    await expect(outsideAnyProject(async () => "done")).resolves.toBe("done");
  });

  it("leaves no temporary file of Cursor's transcript state behind a rename that failed", async () => {
    const storageRoot = await folder("rerail-cursor-state-");
    vi.mocked(rename).mockRejectedValueOnce(Object.assign(new Error("EXDEV: cross-device link not permitted"), { code: "EXDEV" }));
    const session = { host: "cursor", sessionId: "session-1", cwd: storageRoot } satisfies AdapterSession;
    const runtime = {} as RerailEventLoop; // Never reached: a prompt submission only records Cursor's text mode.
    await expect(ingestCursorTranscript({ hook_event_name: "beforeSubmitPrompt" }, session, runtime, storageRoot)).rejects.toThrow(/EXDEV/u);
    const left = (await readdir(storageRoot, { recursive: true })).filter(name => name.endsWith(".tmp"));
    expect(left).toEqual([]);
  });
});

/** A staged package whose CLI answers `state` and then fails `start`, or whose event stream exits 1. */
async function candidate(failing: "cli" | "event-stream"): Promise<Installation> {
  const root = await folder("rerail-candidate-");
  const dist = join(root, "dist");
  await mkdir(join(dist, "cli"), { recursive: true });
  await mkdir(join(dist, "harness"), { recursive: true });
  await writeFile(join(root, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "2.0.0", type: "module" }));
  await writeFile(join(dist, "index.js"), "export function nativeHealth() { return { ok: true }; }\n");
  await writeFile(join(dist, "cli", "cli.js"), [
    'if (process.argv[2] === "state") process.stdout.write("never\\n");',
    `if (process.argv[2] === "start" && ${JSON.stringify(failing === "cli")}) { process.stderr.write("start refused"); process.exit(1); }`,
    "",
  ].join("\n"));
  await writeFile(join(dist, "harness", "event-stream.js"), 'process.stderr.write("stream refused"); process.exit(1);\n');
  return { version: "2.0.0", packageRoot: root };
}

describe("the update's health check", () => {
  it("names the CLI's failure, not the removal of its scratch folder", async () => {
    const installation = await candidate("cli");
    failNextRemoval();
    await expect(validateManagedInstallation(installation)).rejects.toThrow(/Installed CLI could not start a health-check repository/u);
  });

  it("names the event stream's failure, not the removal of its scratch folder", async () => {
    const installation = await candidate("event-stream");
    failNextRemoval();
    await expect(validateManagedInstallation(installation)).rejects.toThrow(/Installed event stream exited 1: stream refused/u);
  });
});
