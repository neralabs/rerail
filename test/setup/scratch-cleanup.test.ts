// The update's health check works in a scratch folder Windows may still hold when it ends: the check's own error is
// the one thrown when removing that folder fails. node:fs/promises is spied on so a test can make one removal fail.
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { validateManagedInstallation, type Installation } from "../../src/setup/updater.js";

vi.mock("node:fs/promises", { spy: true });

const roots: string[] = [];
afterEach(async () => {
  vi.mocked(rm).mockReset();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

/** The next removal fails as Windows fails one while a process holds a file in the folder; the folder goes later. */
function failNextRemoval(): void {
  vi.mocked(rm).mockImplementationOnce(async (path) => {
    roots.push(String(path));
    throw Object.assign(new Error(`EBUSY: resource busy or locked, rmdir '${String(path)}'`), { code: "EBUSY" });
  });
}

/** A staged package whose CLI answers `state` and then fails `start`, or whose event stream exits 1. */
async function candidate(failing: "cli" | "event-stream"): Promise<Installation> {
  const root = await mkdtemp(join(tmpdir(), "rerail-candidate-"));
  roots.push(root);
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
