import { mkdtemp, mkdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, it } from "vitest";
import { loadAssetManifest, selectAsset } from "../src/static-assets.js";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

async function build(): Promise<{ root: string; outside: string }> {
  const base = await realpath(await mkdtemp(join(tmpdir(), "playground-assets-")));
  directories.push(base);
  const root = join(base, "ui");
  const outside = join(base, "secrets");
  await mkdir(join(root, "assets"), { recursive: true });
  await mkdir(outside);
  await Promise.all([
    writeFile(join(root, "index.html"), "<!doctype html>"),
    writeFile(join(root, "assets", "app.js"), "export {};"),
    writeFile(join(root, "notes.txt"), "text"),
    writeFile(join(outside, "id_rsa"), "PRIVATE"),
  ]);
  await symlink(join(outside, "id_rsa"), join(root, "leak.js"));
  await symlink(outside, join(root, "leak"));
  return { root, outside };
}

it("enumerates only the regular files the client emits", async () => {
  const { root } = await build();
  const manifest = await loadAssetManifest(root);
  expect(manifest).toEqual({
    "/index.html": { path: join(root, "index.html"), contentType: "text/html; charset=utf-8" },
    "/assets/app.js": { path: join(root, "assets", "app.js"), contentType: "text/javascript; charset=utf-8" },
  });
});

it("answers built files, client routes, and nothing else", async () => {
  const { root } = await build();
  const manifest = await loadAssetManifest(root);
  expect(selectAsset(manifest, "/assets/app.js")?.path).toBe(join(root, "assets", "app.js"));
  // `/`, a client-side route, and a bogus extension-less path all reload the entry document.
  for (const pathname of ["/", "/rules/some-rule", "/leak/id_rsa", "/assets"]) {
    expect(selectAsset(manifest, pathname)?.path, pathname).toBe(join(root, "index.html"));
  }
  for (const pathname of [
    "/assets/missing.js",
    "/../secrets/id_rsa.js",
    "/assets/../../secrets/id_rsa.js",
    "/%2e%2e/secrets/id_rsa.js",
    "/leak.js",
    "/notes.txt",
  ]) {
    expect(selectAsset(manifest, pathname), pathname).toBeUndefined();
  }
});
