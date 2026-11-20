import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { readCargoWorkspace, readManifest } from "../../../../src/engine/catalog/lint-config/cargo-workspace.js";
import type { LinterNote } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { symlinkOrSkip } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-cargo-workspace-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

/** A package's manifest named `name`, with `rest` after its `[package]` table. */
const crate = (name: string, rest = ""): string => `[package]\nname = "${name}"\n${rest}`;

/** The workspace at `root`: its members' folders, the folders Rerail cannot tell (with why), and the notes. */
function workspaceAt(root: string) {
  const manifest = readManifest(root, "");
  if (manifest === undefined || "why" in manifest) throw new Error("no root manifest read");
  const notes: LinterNote[] = [];
  const found = readCargoWorkspace(root, manifest, notes);
  return {
    members: found.members.map(member => member.folder),
    unread: Object.fromEntries(found.unread.map(({ folder, because }) => [folder, because])),
    notes: notes.map(note => note.text),
  };
}

describe("readCargoWorkspace", () => {
  it("expands member globs as the glob crate does: *, ? and [...] in a name, dot folders included, files left out", async () => {
    const root = await repository({
      "Cargo.toml": "[workspace]\nmembers = [\"crates/*\", \"tools/?x\", \"libs/[!q]*\", \"ext/[a-c]\", \"dots/a.*\", \"nested/*/core\", \"pkg?\"]\n",
      "crates/a/Cargo.toml": crate("a"),
      "crates/.hidden/Cargo.toml": crate("hidden"),
      "crates/README.md": "Crates.\n",
      "tools/tx/Cargo.toml": crate("tx"),
      "libs/util/Cargo.toml": crate("util"),
      "libs/quiet/Cargo.toml": crate("quiet"),
      "ext/b/Cargo.toml": crate("b"),
      "ext/d/Cargo.toml": crate("d"),
      "dots/a.b/Cargo.toml": crate("ab"),
      "dots/axb/Cargo.toml": crate("axb"),
      "nested/one/core/Cargo.toml": crate("core"),
      "nested/file": "",
      "pkgs/Cargo.toml": crate("pkgs"),
    });
    expect(workspaceAt(root)).toEqual({
      members: ["crates/.hidden", "crates/a", "tools/tx", "libs/util", "ext/b", "dots/a.b", "nested/one/core", "pkgs"],
      unread: {
        "dots/axb": "dots/axb/Cargo.toml is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder",
        "ext/d": "ext/d/Cargo.toml is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder",
        "libs/quiet": "libs/quiet/Cargo.toml is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder",
      },
      notes: [
        "dots/axb/Cargo.toml is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder; Clippy's lints there stay as the pack has them.",
        "ext/d/Cargo.toml is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder; Clippy's lints there stay as the pack has them.",
        "libs/quiet/Cargo.toml is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder; Clippy's lints there stay as the pack has them.",
      ],
    });
  });

  it("leaves to the pack the folder of a glob it does not expand, or one Cargo refuses", async () => {
    const root = await repository({
      "Cargo.toml": "[workspace]\nmembers = [\"a/**\", \"b/*/../c\", \"c/[x\", \"none/*\", \"crates/*/*\"]\n",
      "crates/file": "",
    });
    expect(workspaceAt(root)).toEqual({
      members: [],
      unread: {
        a: "Cargo.toml [workspace] members \"a/**\" is not expanded: Rerail expands *, ? and [...] in a folder's name only",
        b: "Cargo.toml [workspace] members \"b/*/../c\" is not expanded: Rerail expands *, ? and [...] in a folder's name only",
        c: "Cargo.toml [workspace] members \"c/[x\" is not a glob Cargo takes: a [ is not closed",
        none: "Cargo.toml [workspace] members \"none/*\" matches nothing, which Cargo refuses",
        // `crates/file` is no folder to look into.
        crates: "Cargo.toml [workspace] members \"crates/*/*\" matches nothing, which Cargo refuses",
      },
      notes: [
        "Cargo.toml [workspace] members \"a/**\" is not expanded: Rerail expands *, ? and [...] in a folder's name only; Clippy's lints there stay as the pack has them.",
        "Cargo.toml [workspace] members \"b/*/../c\" is not expanded: Rerail expands *, ? and [...] in a folder's name only; Clippy's lints there stay as the pack has them.",
        "Cargo.toml [workspace] members \"c/[x\" is not a glob Cargo takes: a [ is not closed; Clippy's lints there stay as the pack has them.",
        "Cargo.toml [workspace] members \"none/*\" matches nothing, which Cargo refuses; Clippy's lints there stay as the pack has them.",
        "Cargo.toml [workspace] members \"crates/*/*\" matches nothing, which Cargo refuses; Clippy's lints there stay as the pack has them.",
      ],
    });
  });

  it("takes a literal member as a path, and notes one outside the repository", async () => {
    const root = await repository({
      "Cargo.toml": crate("root", "\n[workspace]\nmembers = [\"./a/\", \"gone/../b\", \"missing\", \"notes.md\", \"../outside\", \"/absolute\", \".\"]\n"),
      "a/Cargo.toml": crate("a"),
      "b/Cargo.toml": crate("b"),
      "notes.md": "Notes.\n",
    });
    expect(workspaceAt(root)).toEqual({
      members: ["a", "b"],
      unread: { missing: "missing/Cargo.toml is not read: there is none, which Cargo refuses" },
      notes: [
        "Cargo.toml [workspace] members \"../outside\" is outside the repository; it is not read.",
        "Cargo.toml [workspace] members \"/absolute\" is outside the repository; it is not read.",
        "missing/Cargo.toml is not read: there is none, which Cargo refuses; Clippy's lints there stay as the pack has them.",
      ],
    });
  });

  it("leaves out what exclude names, unless a member names it", async () => {
    const root = await repository({
      "Cargo.toml": "[workspace]\nmembers = [\"crates/*\", \"crates/keep\"]\nexclude = [\"crates/b\", \"crates/keep\", \"crates/*\", \"/crates\"]\n",
      "crates/a/Cargo.toml": crate("a"),
      "crates/b/Cargo.toml": crate("b"),
      "crates/keep/Cargo.toml": crate("keep"),
    });
    const { members, unread } = workspaceAt(root);
    expect({ members, unread: Object.keys(unread) }).toEqual({ members: ["crates/a", "crates/keep"], unread: ["crates/b"] });
  });

  it("follows path dependencies from the root package and each member, the workspace's included", async () => {
    const root = await repository({
      "Cargo.toml": crate("root", [
        "",
        "[dependencies]",
        "util = { path = \"util\" }",
        "registry = \"1.0\"",
        "versioned = { version = \"1\" }",
        "outside = { path = \"../elsewhere\" }",
        "itself = { path = \".\" }",
        "shared = { workspace = true }",
        "pinned = { workspace = true }",
        "",
        "[target]",
        "flag = 1",
        "",
        "[target.'cfg(unix)'.dev-dependencies]",
        "unixy = { path = \"./platform/unixy\" }",
        "",
        "[workspace]",
        "",
        "[workspace.dependencies]",
        "shared = { path = \"libs/shared\" }",
        "pinned = \"2\"",
        "",
      ].join("\n")),
      "util/Cargo.toml": crate("util", "\n[build-dependencies]\nhelper = { path = \"../helpers/helper\" }\n"),
      "platform/unixy/Cargo.toml": crate("unixy"),
      "libs/shared/Cargo.toml": crate("shared"),
      "helpers/helper/Cargo.toml": crate("helper"),
    });
    expect(workspaceAt(root).members).toEqual(["util", "helpers/helper", "libs/shared", "platform/unixy"]);
  });

  it("takes no workspace dependency from a [workspace] dependencies that is not a table, and no members without a [workspace]", async () => {
    const odd = await repository({
      "Cargo.toml": crate("root", "\n[dependencies]\nshared = { workspace = true }\n\n[workspace]\ndependencies = 1\n"),
      "shared/Cargo.toml": crate("shared"),
    });
    expect(workspaceAt(odd).members).toEqual([]);
    const alone = await repository({ "Cargo.toml": crate("root", "\n[dependencies]\nutil = { path = \"util\" }\n"), "util/Cargo.toml": crate("util") });
    expect(workspaceAt(alone)).toMatchObject({ members: [], unread: { util: "util/Cargo.toml is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder" } });
  });

  it("refuses a member as Cargo does: no [package], a workspace of its own, another workspace root, or a manifest it cannot read", async () => {
    const root = await repository({
      "Cargo.toml": "[workspace]\nmembers = [\"virtual\", \"own\", \"named-root\", \"named-elsewhere\", \"named-number\", \"broken\"]\n",
      "virtual/Cargo.toml": "[lib]\npath = \"lib.rs\"\n",
      "own/Cargo.toml": crate("own", "\n[workspace]\n"),
      "named-root/Cargo.toml": "[package]\nname = \"n\"\nworkspace = \"..\"\n",
      "named-elsewhere/Cargo.toml": "[package]\nname = \"e\"\nworkspace = \"../own\"\n",
      "named-number/Cargo.toml": "[package]\nname = \"m\"\nworkspace = 1\n",
      "broken/Cargo.toml": "[package\n",
    });
    expect(workspaceAt(root)).toMatchObject({
      members: ["named-root"],
      unread: {
        virtual: "virtual/Cargo.toml is not read: it has no [package], which Cargo refuses for a member",
        own: "own/Cargo.toml is not read: it is the root of a workspace of its own, which Cargo refuses for a member",
        "named-elsewhere": "named-elsewhere/Cargo.toml is not read: its package.workspace names another workspace root, which Cargo refuses",
        "named-number": "named-number/Cargo.toml is not read: its package.workspace names another workspace root, which Cargo refuses",
        broken: "broken/Cargo.toml is not read: line 1: a table header is not closed",
      },
    });
  });

  it("does not read a member's manifest through a link", async context => {
    const root = await repository({ "Cargo.toml": "[workspace]\nmembers = [\"linked\"]\n", "elsewhere/Cargo.toml": crate("x"), "linked/.keep": "" });
    symlinkOrSkip(context, join(root, "elsewhere", "Cargo.toml"), join(root, "linked", "Cargo.toml"));
    expect(workspaceAt(root).unread).toMatchObject({ linked: "linked/Cargo.toml is not read: it is not a regular file of at most 1 MiB in the repository" });
  });
});
