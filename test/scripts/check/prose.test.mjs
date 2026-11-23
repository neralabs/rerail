// scripts/check/ci/prose.mjs: which pull requests change prose alone, so run no Windows shard and no pack dry run.
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { engineSources } from "../../../scripts/check/ci/engine-inputs.mjs";
import { isProse, proseOnly } from "../../../scripts/check/ci/prose.mjs";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));

describe("prose", () => {
  it("is a changeset, a changelog entry, the root's Markdown and docs/'s", () => {
    for (const path of [".changeset/fixed-javascript-curly-using.md", ".changelog/fixed-rules-on-few-threads-never-ending.md", "README.md", "CHANGELOG.md", "docs/release.md", "docs/guides/packs.md"]) {
      expect(isProse(path), path).toBe(true);
    }
  });

  it("is not the rule syntax's Markdown, which the engine and the translation suite read", () => {
    expect(isProse("docs/rule-syntax/README.md")).toBe(false);
  });

  it("is not Markdown that ships in a package or is a test input", () => {
    for (const path of ["packs/python/README.md", "packs/python/CHANGELOG.md", "skills/rerail/SKILL.md", "hooks/README.md", "plugins/x/README.md", "fixtures/blade/licenses/livewire/LICENSE.md", "conformance/README.md"]) {
      expect(isProse(path), path).toBe(false);
    }
  });

  it("holds none of the engine's inputs, which the Windows shards build and load", () => {
    const { dirs, paths } = engineSources(ROOT);
    expect([...paths].filter(isProse)).toEqual([]);
    expect(dirs.filter((dir) => isProse(`${dir}README.md`))).toEqual([]);
  });

  it("is not a changeset's or changelog's configuration, nor a file below their entries", () => {
    for (const path of [".changeset/config.json", ".changeset/sub/x.md", ".changelog/sub/x.md", "docs/site.json", "src/cli/cli.ts"]) {
      expect(isProse(path), path).toBe(false);
    }
  });

  it("alone is every changed file prose, and never an empty list", () => {
    expect(proseOnly([".changeset/a.md", ".changelog/fixed-b.md", "docs/c.md"])).toBe(true);
    expect(proseOnly([".changeset/a.md", "src/cli/cli.ts"])).toBe(false);
    // A rename lists both names: a file moved out of src/ into docs/ is not prose alone.
    expect(proseOnly(["docs/notes.md", "src/notes.md"])).toBe(false);
    expect(proseOnly([])).toBe(false);
  });
});
