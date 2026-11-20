import { execFileSync } from "node:child_process";
import { appendFile, mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IgnoreRuleFiles } from "../../../src/engine/code-rule/ignore-rule-files.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

let home: string;

beforeEach(async () => {
  home = await realpath(await mkdtemp(join(tmpdir(), "rerail-ignore-rule-files-")));
  vi.stubEnv("HOME", home);
  vi.stubEnv("XDG_CONFIG_HOME", "");
  vi.stubEnv("GIT_CONFIG_GLOBAL", join(home, "no-gitconfig"));
  vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
});
afterEach(async () => {
  vi.unstubAllEnvs();
  await removeFolder(home);
});

async function checkout(path: string): Promise<string> {
  await mkdir(path, { recursive: true });
  execFileSync("git", ["init", "--quiet"], { cwd: path });
  return path;
}

const look = { named: [], observed: [] };

describe("IgnoreRuleFiles", () => {
  it("reports what makes an exclude file ignore more, a nested checkout's too, once, and nothing for a comment or a duplicate", async () => {
    const root = await checkout(join(home, "repo"));
    const nested = await checkout(join(root, "vendor", "lib"));
    await appendFile(join(root, ".git", "info", "exclude"), "node_modules\n");
    const files = await IgnoreRuleFiles.open(root, [root, nested]);

    await appendFile(join(nested, ".git", "info", "exclude"), "scratch/\n");
    expect(await files.changes(look, [root, nested])).toEqual([{ path: "vendor/lib/.git/info/exclude", via: "observed", ignoresMore: ["scratch/"] }]);
    expect(await files.changes(look, [root, nested])).toEqual([]);

    // What something outside the repository appends again and again, a comment, a blank line: nothing more is ignored.
    await appendFile(join(root, ".git", "info", "exclude"), "node_modules\n# a note\n\nnode_modules  \n");
    expect(await files.changes(look, [root, nested])).toEqual([]);
  });

  it("reports a negation an exclude file lost, and nothing for a pattern it lost", async () => {
    const root = await checkout(join(home, "repo"));
    await writeFile(join(root, ".git", "info", "exclude"), "*.log\n!keep.log\nout/\n");
    const files = await IgnoreRuleFiles.open(root, [root]);
    await writeFile(join(root, ".git", "info", "exclude"), "*.log\n");
    expect(await files.changes(look, [root])).toEqual([{ path: ".git/info/exclude", via: "observed", ignoresMore: ["!keep.log"] }]);
  });

  it("records a pattern repeated after a negation, and a reorder, in a file with a negation: Git's last match wins", async () => {
    const root = await checkout(join(home, "repo"));
    const exclude = join(root, ".git", "info", "exclude");
    await writeFile(exclude, "*.log\n!keep.log\n");
    const files = await IgnoreRuleFiles.open(root, [root]);
    await appendFile(exclude, "*.log\n");
    expect(await files.changes(look, [root])).toEqual([{ path: ".git/info/exclude", via: "observed", ignoresMore: [], reordered: true }]);
    await writeFile(exclude, "!keep.log\n*.log\n");
    expect(await files.changes(look, [root])).toEqual([{ path: ".git/info/exclude", via: "observed", ignoresMore: [], reordered: true }]);
  });

  it("follows `git config core.excludesFile` to a new file, compared from empty", async () => {
    const root = await checkout(join(home, "repo"));
    const files = await IgnoreRuleFiles.open(root, [root]);
    await writeFile(join(root, "ig"), "cfg/\n");
    execFileSync("git", ["config", "core.excludesFile", "ig"], { cwd: root });
    expect(await files.changes(look, [root])).toEqual([{ path: "ig", via: "observed", ignoresMore: ["cfg/"] }]);
    expect(await files.changes(look, [root])).toEqual([]);
  });

  it("compares a .gitignore a tool edit names with what it held before the edit, and one a walk found with what it first held", async () => {
    const root = await checkout(join(home, "repo"));
    const files = await IgnoreRuleFiles.open(root, [root]);
    await writeFile(join(root, ".gitignore"), "dist/\n");
    await files.remember([join(root, ".gitignore")]);

    await appendFile(join(root, ".gitignore"), "# why\ndist/\n");
    expect(await files.changes({ named: [join(root, ".gitignore"), "src/a.ts"], observed: [] }, [root])).toEqual([]);
    await appendFile(join(root, ".gitignore"), "scratch/\n");
    expect(await files.changes({ named: [".gitignore"], observed: [] }, [root])).toEqual([{ path: ".gitignore", via: "tool", ignoresMore: ["scratch/"] }]);

    // A .gitignore no walk saw before, that ignores itself and all beside it.
    await mkdir(join(root, "hidden"));
    await writeFile(join(root, "hidden", ".gitignore"), "*\n");
    expect(await files.changes({ named: [], observed: ["hidden/.gitignore"] }, [root])).toEqual([{ path: "hidden/.gitignore", via: "observed", ignoresMore: ["*"] }]);
  });

  it("watches a checkout that appears later from what its exclude file holds then", async () => {
    const root = await checkout(join(home, "repo"));
    const files = await IgnoreRuleFiles.open(root, [root]);
    const later = await checkout(join(root, "later"));
    await appendFile(join(later, ".git", "info", "exclude"), "before-watched/\n");
    expect(await files.changes(look, [root, later])).toEqual([]);
    await appendFile(join(later, ".git", "info", "exclude"), "after-watched/\n");
    expect(await files.changes(look, [root, later])).toEqual([{ path: "later/.git/info/exclude", via: "observed", ignoresMore: ["after-watched/"] }]);
  });

  it("reports Git's default excludes file, outside the session folder, by its absolute path", async () => {
    const root = await checkout(join(home, "repo"));
    const files = await IgnoreRuleFiles.open(root, [root]);
    await mkdir(join(home, ".config", "git"), { recursive: true });
    await appendFile(join(home, ".config", "git", "ignore"), "*.local\n");
    expect(await files.changes(look, [root])).toEqual([{ path: join(home, ".config", "git", "ignore"), via: "observed", ignoresMore: ["*.local"] }]);
  });
});
