import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, it } from "vitest";
import { ompExecutable } from "../src/process.js";

const directories: string[] = [];
const previous = process.env.RERAIL_OMP_EXECUTABLE;
afterEach(async () => {
  if (previous === undefined) delete process.env.RERAIL_OMP_EXECUTABLE;
  else process.env.RERAIL_OMP_EXECUTABLE = previous;
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

it("refuses an unset or relative host before any worker is spawned", () => {
  delete process.env.RERAIL_OMP_EXECUTABLE;
  expect(() => ompExecutable()).toThrow("absolute standalone OMP executable");
  process.env.RERAIL_OMP_EXECUTABLE = "omp";
  expect(() => ompExecutable()).toThrow("absolute standalone OMP executable");
});

/** Reinstalling rerail/ deletes the pinned host under node_modules/.cache while the supervisor holds its path. */
it("names the vanished pinned host instead of letting sandbox-exec fail with a bare status", async () => {
  const root = await mkdtemp(join(tmpdir(), "playground-omp-"));
  directories.push(root);
  const executable = join(root, "omp");
  await writeFile(executable, "#!/bin/sh\n");
  await chmod(executable, 0o755);
  process.env.RERAIL_OMP_EXECUTABLE = executable;
  expect(ompExecutable()).toBe(executable);

  await rm(executable);
  expect(() => ompExecutable()).toThrow(/no longer executable[\s\S]*restart just playground/);
});
