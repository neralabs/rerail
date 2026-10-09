import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { normalizeRemoteAddress, readCloneId, readRemoteAddress, resolveCloneId } from "../src/repository-identity.js";

/** When set, the next `mkdirSync` of a `.rerail` folder loses a race: another process creates it first. */
const race = vi.hoisted(() => ({ armed: false }));
vi.mock("node:fs", async importOriginal => {
  const fs = await importOriginal<typeof import("node:fs")>();
  const mkdirSync = ((path: Parameters<typeof fs.mkdirSync>[0], options?: Parameters<typeof fs.mkdirSync>[1]) => {
    if (race.armed && String(path).endsWith(".rerail")) {
      race.armed = false;
      fs.mkdirSync(path, options);
      throw Object.assign(new Error(`EEXIST: file already exists, mkdir '${String(path)}'`), { code: "EEXIST" });
    }
    return fs.mkdirSync(path, options);
  }) as typeof fs.mkdirSync;
  return { ...fs, mkdirSync, default: { ...fs, mkdirSync } };
});

describe("normalizeRemoteAddress", () => {
  it("gives the ssh, https and differently cased addresses of one repository the same key", () => {
    const keys = [
      "git@github.com:acme/data-layer.git",
      "https://github.com/acme/data-layer.git",
      "https://github.com/Acme/data-layer",
    ].map(normalizeRemoteAddress);
    expect(keys).toEqual(Array(3).fill("remote:github.com/acme/data-layer"));
  });

  it("drops the user, the port, doubled and trailing slashes", () => {
    expect(normalizeRemoteAddress("ssh://git@github.com:22/acme//data-layer.git/")).toBe("remote:github.com/acme/data-layer");
    expect(normalizeRemoteAddress("https://user:token@gitlab.example.com:8443/team/web/")).toBe("remote:gitlab.example.com/team/web");
  });

  it("reads the scp form as host/path and keeps an ssh Host alias distinct", () => {
    expect(normalizeRemoteAddress("github.com:acme/web.git")).toBe("remote:github.com/acme/web");
    expect(normalizeRemoteAddress("github-work:acme/web")).toBe("remote:github-work/acme/web");
  });

  it("gives no key for a local path, a file URL or a Windows drive path", () => {
    expect(normalizeRemoteAddress("/srv/git/web.git")).toBeUndefined();
    expect(normalizeRemoteAddress("../web")).toBeUndefined();
    expect(normalizeRemoteAddress("file:///srv/git/web.git")).toBeUndefined();
    expect(normalizeRemoteAddress(String.raw`C:\repos\web`)).toBeUndefined();
    expect(normalizeRemoteAddress("C:/repos/web")).toBeUndefined();
  });
});

describe("readRemoteAddress", () => {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });

  const repository = (...remotes: [string, string][]): string => {
    const root = mkdtempSync(join(tmpdir(), "rerail-remote-"));
    roots.push(root);
    execFileSync("git", ["init", "-q", root]);
    for (const [name, url] of remotes) execFileSync("git", ["-C", root, "remote", "add", name, url]);
    return root;
  };

  it("prefers origin over any other remote", () => {
    const root = repository(["upstream", "https://github.com/acme/upstream.git"], ["origin", "git@git.example.com:acme/web.git"]);
    expect(readRemoteAddress(root)).toEqual({ name: "origin", url: "git@git.example.com:acme/web.git" });
  });

  it("uses the only remote when it is not called origin", () => {
    const root = repository(["upstream", "https://github.com/acme/web.git"]);
    expect(readRemoteAddress(root)).toEqual({ name: "upstream", url: "https://github.com/acme/web.git" });
  });

  it("gives none with two remotes and no origin, or with no remote", () => {
    expect(readRemoteAddress(repository(["a", "https://github.com/acme/a.git"], ["b", "https://github.com/acme/b.git"]))).toBeUndefined();
    expect(readRemoteAddress(repository())).toBeUndefined();
  });
});

describe("clone identity files", () => {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });
  const folder = (): string => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-identity-")));
    roots.push(root);
    return root;
  };

  it("refuses an identity that is a symlink, malformed, or kept in a symlinked .rerail, and never follows one", () => {
    const outside = folder();
    writeFileSync(join(outside, "secret"), "git:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\n");

    const linkedFile = folder();
    mkdirSync(join(linkedFile, ".rerail"));
    symlinkSync(join(outside, "secret"), join(linkedFile, ".rerail", "repository-id"));
    expect(readCloneId(linkedFile)).toBeUndefined();
    expect(() => resolveCloneId(linkedFile)).toThrow(/not a regular file/u);

    const malformed = folder();
    mkdirSync(join(malformed, ".rerail"));
    writeFileSync(join(malformed, ".rerail", "repository-id"), "../../etc\n");
    expect(readCloneId(malformed)).toBeUndefined();
    expect(() => resolveCloneId(malformed)).toThrow(/malformed/u);

    const linkedDirectory = folder();
    symlinkSync(outside, join(linkedDirectory, ".rerail"));
    expect(() => resolveCloneId(linkedDirectory)).toThrow(/not a directory/u);
    expect(readdirSync(outside)).toEqual(["secret"]);
  });

  it("creates its identity when another process creates .rerail at the same moment", () => {
    const root = folder();
    race.armed = true;
    const { cloneId } = resolveCloneId(root);
    expect(race.armed).toBe(false);
    expect(resolveCloneId(root).cloneId).toBe(cloneId);
  });

  it("creates a well-formed id once and reads the same id back", () => {
    const root = folder();
    const { cloneId } = resolveCloneId(root);
    expect(cloneId).toMatch(/^[0-9a-f-]{36}$/u);
    expect(resolveCloneId(root).cloneId).toBe(cloneId);
    expect(readCloneId(root)).toBe(cloneId);
  });
});
