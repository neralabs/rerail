import { afterEach, describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { addLauncherToPath, removeLauncherFromPath } from "../../src/setup/shell-path.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function machine(shell: string) {
  const home = await mkdtemp(join(tmpdir(), "rerail-shell-path-"));
  temporaryDirectories.push(home);
  // A space and a quote: the block must quote the directory for the shell.
  const binDir = join(home, "rerail home's", "bin");
  const env = { SHELL: shell, PATH: "/usr/bin:/bin" };
  return { home, binDir, env, options: { binDir, home, env, platform: "linux" as const } };
}

describe.skipIf(process.platform === "win32")("launcher on PATH", () => {
  it("puts the directory on a new zsh-sourced PATH once, keeping what the file had", async () => {
    const { home, binDir, options } = await machine("/bin/zsh");
    const zshrc = join(home, ".zshrc");
    await writeFile(zshrc, "alias ll='ls -l'");

    expect(await addLauncherToPath(options)).toEqual({ status: "changed", locations: [zshrc] });
    expect(await addLauncherToPath(options)).toEqual({ status: "unchanged", locations: [zshrc] });
    const text = await readFile(zshrc, "utf8");
    expect(text.startsWith("alias ll='ls -l'\n\n# >>> rerail >>>\n")).toBe(true);

    // Sourced twice, as a login file sourcing the rc file does: the directory is on PATH once, first.
    const path = execFileSync("sh", ["-c", `. "$1"; . "$1"; printf %s "$PATH"`, "sh", zshrc], { env: { PATH: "/usr/bin:/bin" }, encoding: "utf8" });
    expect(path).toBe(`${binDir}:/usr/bin:/bin`);

    expect(await removeLauncherFromPath(options)).toEqual({ status: "changed", locations: [zshrc] });
    expect(await readFile(zshrc, "utf8")).toBe("alias ll='ls -l'\n");
  });

  it("falls back to the account's login shell when SHELL is unset, as inside some coding-agent hosts", async () => {
    const { home, options } = await machine("/bin/zsh");
    const env = { PATH: "/usr/bin:/bin" };
    expect(await addLauncherToPath({ ...options, env, loginShell: "/bin/zsh" })).toEqual({ status: "changed", locations: [join(home, ".zshrc")] });
    expect(existsSync(join(home, ".profile"))).toBe(false);
    // With no login shell known either, the file every POSIX login shell but zsh reads.
    const other = await machine("/bin/zsh");
    expect(await addLauncherToPath({ ...other.options, env, loginShell: null })).toEqual({ status: "changed", locations: [join(other.home, ".profile")] });
  });

  it("writes nothing when the running PATH has the directory, or RERAIL_MODIFY_PATH=0", async () => {
    const { home, binDir, env, options } = await machine("/bin/zsh");
    expect(await addLauncherToPath({ ...options, env: { ...env, PATH: `/usr/bin:${binDir}/` } })).toEqual({ status: "skipped" });
    expect(await addLauncherToPath({ ...options, env: { ...env, RERAIL_MODIFY_PATH: "0" } })).toEqual({ status: "skipped" });
    expect(existsSync(join(home, ".zshrc"))).toBe(false);
  });

  it("replaces a block written for another Rerail home", async () => {
    const { home, options } = await machine("/bin/zsh");
    const zshrc = join(home, ".zshrc");
    await addLauncherToPath({ ...options, binDir: join(home, "old", "bin") });
    await writeFile(zshrc, `${await readFile(zshrc, "utf8")}export EDITOR=vi\n`);

    expect((await addLauncherToPath(options)).status).toBe("changed");
    const text = await readFile(zshrc, "utf8");
    expect(text).not.toContain(join(home, "old", "bin"));
    expect(text.match(/# >>> rerail >>>/gu)).toHaveLength(1);
    expect(text.endsWith("export EDITOR=vi\n")).toBe(true);
  });

  it("covers bash's rc file and the login file bash reads, without creating one that hides .profile", async () => {
    const { home, options } = await machine("/usr/local/bin/bash");
    await writeFile(join(home, ".profile"), "umask 022\n");

    expect(await addLauncherToPath(options)).toEqual({ status: "changed", locations: [join(home, ".bashrc"), join(home, ".profile")] });
    expect(existsSync(join(home, ".bash_profile"))).toBe(false);

    await removeLauncherFromPath(options);
    expect(await readFile(join(home, ".profile"), "utf8")).toBe("umask 022\n");
    expect(await readFile(join(home, ".bashrc"), "utf8")).toBe("");
  });

  it("gives fish its own conf.d file, which removal deletes", async () => {
    const { home, binDir, options } = await machine("/opt/homebrew/bin/fish");
    const config = join(home, ".config", "fish", "conf.d", "rerail.fish");

    expect(await addLauncherToPath(options)).toEqual({ status: "changed", locations: [config] });
    expect(await readFile(config, "utf8")).toContain(`set -gx PATH '${binDir.replaceAll("'", "\\'")}' $PATH`);

    await removeLauncherFromPath(options);
    expect(existsSync(config)).toBe(false);
  });

  it("plans without writing on a dry run, and reports a file it cannot write", async () => {
    const { home, options } = await machine("/bin/sh");
    expect(await addLauncherToPath({ ...options, dryRun: true })).toEqual({ status: "changed", locations: [join(home, ".profile")] });
    expect(existsSync(join(home, ".profile"))).toBe(false);

    await mkdir(join(home, ".profile"));
    expect((await addLauncherToPath(options)).status).toBe("failed");
  });

  it("takes out only the block's own lines, whatever the person did around it", async () => {
    const { home, options } = await machine("/bin/zsh");
    const zshrc = join(home, ".zshrc");
    await addLauncherToPath(options);
    const block = (await readFile(zshrc, "utf8")).trim();

    // The blank line before it removed: the lines either side stay two lines.
    await writeFile(zshrc, `a\n${block}\nb\n`);
    await removeLauncherFromPath(options);
    expect(await readFile(zshrc, "utf8")).toBe("a\nb\n");

    // A start marker left on its own reaches no further than itself, and the real block after it is the one replaced.
    await writeFile(zshrc, `# >>> rerail >>>\nkeep me\n${block}\n`);
    await removeLauncherFromPath(options);
    expect(await readFile(zshrc, "utf8")).toBe("# >>> rerail >>>\nkeep me\n");
  });

  it("takes the block out of a CRLF file without joining the line above it", async () => {
    const { home, options } = await machine("/bin/zsh");
    const zshrc = join(home, ".zshrc");
    await addLauncherToPath(options);
    const block = (await readFile(zshrc, "utf8")).trim().replaceAll("\n", "\r\n");
    await writeFile(zshrc, `a\r\n${block}\r\nb\r\n`);
    await removeLauncherFromPath(options);
    expect(await readFile(zshrc, "utf8")).toBe("a\r\nb\r\n");
  });

  it("rewrites a block for a launcher whose path has a $ in it as written", async () => {
    const { home, options } = await machine("/bin/zsh");
    await addLauncherToPath({ ...options, binDir: join(home, "old", "bin") });
    const binDir = join(home, "a$&b", "bin");
    await addLauncherToPath({ ...options, binDir });
    // Quoted for the shell, and the `$&` kept as it is rather than replaced by the matched text.
    expect(await readFile(join(home, ".zshrc"), "utf8")).toContain(`*:'${binDir}':*) ;;`);
  });

  it("finds its block in a CRLF file, so re-running adds no second one", async () => {
    const { home, options } = await machine("/bin/zsh");
    const zshrc = join(home, ".zshrc");
    await addLauncherToPath(options);
    await writeFile(zshrc, (await readFile(zshrc, "utf8")).replaceAll("\n", "\r\n"));
    await addLauncherToPath(options);
    expect((await readFile(zshrc, "utf8")).match(/# >>> rerail >>>/gu)).toHaveLength(1);
  });

  it("reads an empty ZDOTDIR as unset, never as the current folder", async () => {
    const { home, env, options } = await machine("/bin/zsh");
    expect(await addLauncherToPath({ ...options, env: { ...env, ZDOTDIR: "" } })).toEqual({ status: "changed", locations: [join(home, ".zshrc")] });
  });
});
