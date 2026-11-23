import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runRemoteCommand } from "../../../../src/cli/index.js";
import { lookupCheckout, recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { learnRemoteAddress, normalizeRemoteAddress, readRemoteAddress, type RemoteAddress } from "../../../../src/engine/catalog/repository-identity.js";
import { runGit } from "../../../../src/engine/git.js";
import { standingGitAccess } from "../../../../src/engine/native/git-access.js";
import { loadNativeBinding, type NativeGitAccess } from "../../../../src/engine/native/native.js";
import { openRerailDatabase } from "../../../../src/engine/storage/database.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit) };
});

vi.mock("../../../../src/engine/native/git-access.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/native/git-access.js")>();
  return { ...actual, standingGitAccess: vi.fn(actual.standingGitAccess) };
});

vi.mock("../../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/native/native.js")>();
  return { ...actual, loadNativeBinding: vi.fn(actual.loadNativeBinding) };
});

const folders: string[] = [];
afterEach(() => {
  vi.unstubAllEnvs();
  vi.mocked(standingGitAccess).mockReset();
  vi.mocked(loadNativeBinding).mockReset();
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});

/** A temporary folder, its main checkout with one commit on `main`, and where the case's own config files go. */
interface Clone {
  readonly root: string;
  readonly main: string;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

function clone(): Clone {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-kept-remote-")));
  folders.push(root);
  const main = join(root, "main");
  mkdirSync(main);
  git(main, "init", "--quiet", "-b", "main");
  git(main, "commit", "--quiet", "--allow-empty", "-m", "one");
  return { root, main };
}

function write(path: string, text: string): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
}

/** A path as a config file names it: Git reads a backslash in a value as an escape. */
function named(path: string): string {
  return path.replaceAll("\\", "/");
}

const ONE = "https://example.invalid/team/one.git";
const TWO = "https://example.invalid/team/two.git";

function origin(url: string): string {
  return `[remote "origin"]\n\turl = ${url}\n`;
}

/** The remote Git names for `root` now, asked as `main` asks it: `origin`'s address, else the only remote's. */
function gitsRemote(root: string): RemoteAddress | undefined {
  const text = (...args: string[]): string | undefined => {
    const answer = spawnSync("git", ["-C", root, ...args], { encoding: "utf8" });
    return answer.status === 0 ? answer.stdout.trim() : undefined;
  };
  const url = text("ls-remote", "--get-url", "origin");
  if (url !== undefined && url !== "" && url !== "origin") return { name: "origin", url };
  const names = (text("remote") ?? "").split("\n").filter(name => name !== "");
  const only = names.length === 1 ? text("remote", "get-url", "--", names[0]!) : undefined;
  return only === undefined || only === "" ? undefined : { name: names[0]!, url: only };
}

/** The system file of the case's own, which Git reads in place of the machine's. */
function ownSystemFile(at: Clone, text: string): string {
  const system = join(at.root, "system.gitconfig");
  write(system, text);
  vi.stubEnv("GIT_CONFIG_NOSYSTEM", undefined);
  vi.stubEnv("GIT_CONFIG_SYSTEM", system);
  return system;
}

interface Row {
  readonly what: string;
  /** Sets the fact up with a remote Git names; the checkout whose remote is read. */
  readonly arrange: (at: Clone) => string;
  /** Changes the fact, so that Git names another remote. */
  readonly change: (at: Clone) => void;
}

const ROWS: readonly Row[] = [
  {
    what: "a remote set only in the system file",
    arrange: at => {
      ownSystemFile(at, origin(ONE));
      return at.main;
    },
    change: at => write(join(at.root, "system.gitconfig"), origin(TWO)),
  },
  {
    what: "an address the system file rewrites with url.<base>.insteadOf",
    arrange: at => {
      git(at.main, "remote", "add", "origin", ONE);
      ownSystemFile(at, "[core]\n");
      return at.main;
    },
    change: at => write(join(at.root, "system.gitconfig"), "[url \"https://mirror.invalid/\"]\n\tinsteadOf = https://example.invalid/\n"),
  },
  {
    what: "a file the system file includes",
    arrange: at => {
      write(join(at.root, "included"), origin(ONE));
      ownSystemFile(at, `[include]\n\tpath = ${named(join(at.root, "included"))}\n`);
      return at.main;
    },
    change: at => write(join(at.root, "included"), origin(TWO)),
  },
  {
    what: "a system file GIT_CONFIG_SYSTEM names by a relative path, which Git opens from the checkout",
    arrange: at => {
      write(join(at.main, "relative.gitconfig"), origin(ONE));
      vi.stubEnv("GIT_CONFIG_NOSYSTEM", undefined);
      vi.stubEnv("GIT_CONFIG_SYSTEM", "relative.gitconfig");
      return at.main;
    },
    change: at => write(join(at.main, "relative.gitconfig"), origin(TWO)),
  },
  {
    what: "the global file GIT_CONFIG_GLOBAL names",
    arrange: at => {
      write(join(at.root, "global"), origin(ONE));
      vi.stubEnv("GIT_CONFIG_GLOBAL", join(at.root, "global"));
      return at.main;
    },
    change: at => write(join(at.root, "global"), origin(TWO)),
  },
  {
    what: "a global file GIT_CONFIG_GLOBAL names by a relative path, which Git opens from the checkout",
    arrange: at => {
      write(join(at.main, "relative.gitconfig"), origin(ONE));
      vi.stubEnv("GIT_CONFIG_GLOBAL", "relative.gitconfig");
      return at.main;
    },
    change: at => write(join(at.main, "relative.gitconfig"), origin(TWO)),
  },
  {
    what: "the global file in XDG_CONFIG_HOME, with HOME's beside it",
    arrange: at => {
      write(join(at.root, "xdg", "git", "config"), origin(ONE));
      vi.stubEnv("GIT_CONFIG_GLOBAL", undefined);
      vi.stubEnv("HOME", join(at.root, "home"));
      vi.stubEnv("XDG_CONFIG_HOME", join(at.root, "xdg"));
      return at.main;
    },
    change: at => write(join(at.root, "xdg", "git", "config"), origin(TWO)),
  },
  {
    what: "the repository's config",
    arrange: at => {
      git(at.main, "remote", "add", "origin", ONE);
      return at.main;
    },
    change: at => git(at.main, "remote", "set-url", "origin", TWO),
  },
  {
    what: "a linked worktree's config.worktree",
    arrange: at => {
      git(at.main, "config", "extensions.worktreeConfig", "true");
      git(at.main, "worktree", "add", "--quiet", "-b", "linked", join(at.root, "linked"));
      git(join(at.root, "linked"), "config", "--worktree", "remote.origin.url", ONE);
      return join(at.root, "linked");
    },
    change: at => git(join(at.root, "linked"), "config", "--worktree", "remote.origin.url", TWO),
  },
  {
    what: "a file the repository's config includes with include.path",
    arrange: at => {
      write(join(at.root, "included"), origin(ONE));
      git(at.main, "config", "include.path", named(join(at.root, "included")));
      return at.main;
    },
    change: at => write(join(at.root, "included"), origin(TWO)),
  },
  {
    what: "a file the repository's config includes with includeIf",
    arrange: at => {
      write(join(at.root, "included"), origin(ONE));
      appendFileSync(join(at.main, ".git", "config"), `[includeIf "onbranch:main"]\n\tpath = ${named(join(at.root, "included"))}\n`);
      return at.main;
    },
    change: at => write(join(at.root, "included"), origin(TWO)),
  },
  {
    what: "a file included by a section that opens after another on its line",
    arrange: at => {
      write(join(at.root, "included"), origin(ONE));
      appendFileSync(join(at.main, ".git", "config"), `[alias] [include] path = ${named(join(at.root, "included"))}\n`);
      return at.main;
    },
    change: at => write(join(at.root, "included"), origin(TWO)),
  },
  {
    what: "a file GIT_CONFIG_COUNT's keys include",
    arrange: at => {
      write(join(at.root, "included"), origin(ONE));
      vi.stubEnv("GIT_CONFIG_COUNT", "1");
      vi.stubEnv("GIT_CONFIG_KEY_0", "include.path");
      vi.stubEnv("GIT_CONFIG_VALUE_0", join(at.root, "included"));
      return at.main;
    },
    change: at => write(join(at.root, "included"), origin(TWO)),
  },
  {
    what: "a remote GIT_CONFIG_COUNT's keys set",
    arrange: at => {
      vi.stubEnv("GIT_CONFIG_COUNT", "1");
      vi.stubEnv("GIT_CONFIG_KEY_0", "remote.origin.url");
      vi.stubEnv("GIT_CONFIG_VALUE_0", ONE);
      return at.main;
    },
    change: () => vi.stubEnv("GIT_CONFIG_VALUE_0", TWO),
  },
  {
    what: "the count GIT_CONFIG_COUNT gives its keys",
    arrange: at => {
      vi.stubEnv("GIT_CONFIG_COUNT", "1");
      vi.stubEnv("GIT_CONFIG_KEY_0", "remote.origin.url");
      vi.stubEnv("GIT_CONFIG_VALUE_0", ONE);
      return at.main;
    },
    change: () => vi.stubEnv("GIT_CONFIG_COUNT", "0"),
  },
  {
    what: "a key GIT_CONFIG_COUNT's keys name",
    arrange: at => {
      vi.stubEnv("GIT_CONFIG_COUNT", "1");
      vi.stubEnv("GIT_CONFIG_KEY_0", "remote.origin.url");
      vi.stubEnv("GIT_CONFIG_VALUE_0", ONE);
      return at.main;
    },
    change: () => vi.stubEnv("GIT_CONFIG_KEY_0", "remote.other.url"),
  },
  {
    what: "a remote GIT_CONFIG_PARAMETERS sets",
    arrange: at => {
      vi.stubEnv("GIT_CONFIG_PARAMETERS", `'remote.origin.url=${ONE}'`);
      return at.main;
    },
    change: () => vi.stubEnv("GIT_CONFIG_PARAMETERS", `'remote.origin.url=${TWO}'`),
  },
  {
    what: "a remote in the common Git folder's remotes file, Git's older form",
    arrange: at => {
      write(join(at.main, ".git", "remotes", "origin"), `URL: ${ONE}\n`);
      return at.main;
    },
    change: at => write(join(at.main, ".git", "remotes", "origin"), `URL: ${TWO}\n`),
  },
  {
    what: "a remote in the common Git folder's branches file, Git's oldest form",
    arrange: at => {
      write(join(at.main, ".git", "branches", "origin"), `${ONE}\n`);
      return at.main;
    },
    change: at => write(join(at.main, ".git", "branches", "origin"), `${TWO}\n`),
  },
];

describe("a clone's remote kept by the daemon, against Git's own answer once a fact it rests on changes", () => {
  it.each(ROWS)("gives Git's new remote at the next record once $what changes", async ({ arrange, change }) => {
    const at = clone();
    const checkout = arrange(at);
    const before = gitsRemote(checkout);
    expect(before?.url).toBeDefined();
    await learnRemoteAddress(checkout);
    expect(readRemoteAddress(checkout)).toEqual(before);

    change(at);
    const after = gitsRemote(checkout);
    // Git reads the fact: its answer moved.
    expect(after).not.toEqual(before);
    expect(readRemoteAddress(checkout)).toEqual(after);
  });
});

describe("the remotes kept for a clone's worktrees", () => {
  it("keeps the main checkout's, read with a config.worktree of its own, apart from the one its other worktrees share", () => {
    const at = clone();
    const linked = join(at.root, "linked");
    git(at.main, "config", "extensions.worktreeConfig", "true");
    git(at.main, "config", "--worktree", "rerail.case", "main");
    git(at.main, "worktree", "add", "--quiet", "-b", "linked", linked);
    expect(remoteAsks(() => [readRemoteAddress(at.main), readRemoteAddress(linked)])).toBeGreaterThan(0);

    // Read in turn, each answer stands for its own facts.
    expect(remoteAsks(() => [readRemoteAddress(at.main), readRemoteAddress(linked), readRemoteAddress(at.main)])).toBe(0);
  });
});

/** The engine's owner of its Git reads, naming `files` as Rerail's Git reported them. */
function gitAccess(files: Pick<NativeGitAccess, "systemConfig" | "installationConfig">): NativeGitAccess {
  return { ...files, openGitIgnoreMatcher: () => null };
}

/** How many Git processes asked a remote while `act` ran. */
function remoteAsks(act: () => unknown): number {
  const before = vi.mocked(runGit).mock.calls.filter(([, args]) => args.includes("ls-remote")).length;
  act();
  return vi.mocked(runGit).mock.calls.filter(([, args]) => args.includes("ls-remote")).length - before;
}

describe("the system files a kept remote rests on, as the engine's owner of its Git reads names them", () => {
  it.each([
    { what: "the system file Rerail's Git was built to read", files: (file: string) => ({ systemConfig: file, installationConfig: null }) },
    { what: "the file Apple Git reads beside it", files: (file: string, beside: string) => ({ systemConfig: beside, installationConfig: file }) },
  ])("asks Git again once $what changes, with no variable naming another", ({ files }) => {
    const at = clone();
    const file = join(at.root, "named.gitconfig");
    write(file, "[core]\n");
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", undefined);
    vi.stubEnv("GIT_CONFIG_SYSTEM", undefined);
    vi.mocked(standingGitAccess).mockReturnValue(gitAccess(files(file, join(at.root, "built.gitconfig"))));
    expect(remoteAsks(() => readRemoteAddress(at.main))).toBeGreaterThan(0);
    expect(remoteAsks(() => readRemoteAddress(at.main))).toBe(0);

    write(file, "[core]\n\tbare = false\n");
    expect(remoteAsks(() => expect(readRemoteAddress(at.main)).toEqual(gitsRemote(at.main)))).toBeGreaterThan(0);
  });

  it("asks Git each time while Rerail's Git named no system file, and keeps the answer once GIT_CONFIG_NOSYSTEM turns system files off", () => {
    const at = clone();
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", undefined);
    vi.stubEnv("GIT_CONFIG_SYSTEM", undefined);
    vi.mocked(standingGitAccess).mockReturnValue(gitAccess({ systemConfig: null, installationConfig: null }));
    readRemoteAddress(at.main);
    expect(remoteAsks(() => readRemoteAddress(at.main))).toBeGreaterThan(0);

    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "true");
    readRemoteAddress(at.main);
    expect(remoteAsks(() => readRemoteAddress(at.main))).toBe(0);
  });

  it("is Git's answer, for a checkout's record and lookup and for rerail remote too, where no engine loads", async () => {
    const at = clone();
    git(at.main, "remote", "add", "origin", ONE);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", undefined);
    vi.stubEnv("GIT_CONFIG_SYSTEM", undefined);
    vi.mocked(loadNativeBinding).mockImplementation(() => {
      throw new Error("Unsupported platform or missing native addon");
    });
    const key = normalizeRemoteAddress(ONE);
    await learnRemoteAddress(at.main);
    expect(readRemoteAddress(at.main)).toEqual(gitsRemote(at.main));
    const database = openRerailDatabase(join(at.root, "rerail.sqlite"));
    try {
      expect(recordCheckout(database, at.main).repositoryKey).toBe(key);
      expect(lookupCheckout(database, at.main)?.repositoryKey).toBe(key);
    } finally {
      database.$client.close();
    }
    const home = join(at.root, "home");
    mkdirSync(home);
    const printed: string[] = [];
    const stdout = { write: (text: string | Uint8Array): boolean => printed.push(String(text)) > 0 };
    expect(await runRemoteCommand(["exclude", "--repo", at.main], { home, stdout })).toBe(0);
    expect(printed.join("")).toContain(`of ${key}`);
  });
});
