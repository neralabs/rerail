// release-version.mjs run for real, in a throwaway git repository holding a copy of the release scripts: it reads its
// root from its own path, compares rules across git history, and runs at the top level, so nothing short of this
// reaches that comparison. `version:packs` is a stub that bumps every pack, as `changeset version` does for the packs a
// changeset names.
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const BUMP = `import { readdirSync, readFileSync, writeFileSync } from "node:fs";
for (const id of readdirSync("packs")) {
  const path = \`packs/\${id}/package.json\`;
  writeFileSync(path, JSON.stringify({ ...JSON.parse(readFileSync(path, "utf8")), version: "0.1.1" }, null, 2) + "\\n");
}
`;

const directories = [];
afterEach(() => {
  for (const directory of directories.splice(0)) rmSync(directory, { recursive: true, force: true });
});

const packName = (id) => `@neralabs/rerail-pack-${id}`;

/** A repository with the release scripts, the stub and a pending changeset, and helpers to write and commit in it. */
function repository() {
  const root = mkdtempSync(join(tmpdir(), "release-version-"));
  directories.push(root);
  const write = (path, text) => {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  };
  const git = (...args) => execFileSync("git", ["-c", "user.name=test", "-c", "user.email=test@example.invalid", ...args], { cwd: root, encoding: "utf8" });
  cpSync(join(REPOSITORY, "scripts/release"), join(root, "scripts/release"), { recursive: true });
  cpSync(join(REPOSITORY, "scripts/check/check-internal-refs.mjs"), join(root, "scripts/check/check-internal-refs.mjs"));
  write("package.json", `${JSON.stringify({ name: "core", version: "1.0.0", type: "module", scripts: { "version:packs": "node bump.mjs" } }, null, 2)}\n`);
  write("bump.mjs", BUMP);
  // The scripts' dependencies; ignored, so no commit below takes the link.
  symlinkSync(join(REPOSITORY, "node_modules"), join(root, "node_modules"));
  write(".gitignore", "node_modules\n");
  write(".changeset/changed-release.md", '---\n"@neralabs/rerail-pack-any": patch\n---\n');
  git("init", "-q");
  return { root, write, git };
}

/** Writes pack `id` at 0.1.0 with the rules whose ids are `rules`, each starting in Steer. */
function writePack({ write }, id, rules) {
  write(`packs/${id}/package.json`, `${JSON.stringify({ name: packName(id), version: "0.1.0" }, null, 2)}\n`);
  write(`packs/${id}/pack.json`, `${JSON.stringify({ rules: rules.map((rule) => ({ id: rule, document: `rules/${rule}.json` })) }, null, 2)}\n`);
  for (const rule of rules) write(`packs/${id}/rules/${rule}.json`, `${JSON.stringify({ mode: "steer", description: `What ${rule} wants.` })}\n`);
}

const commit = ({ git }, message) => {
  git("add", "-A");
  git("commit", "-q", "-m", message);
};

function releaseVersion({ root }) {
  execFileSync("node", ["scripts/release/release-version.mjs", join(root, "body.md")], { cwd: root, stdio: "pipe" });
}

describe("release-version.mjs, the pull request's description", () => {
  it("ends what it releases with the checklist to tick before merging, every box unticked", () => {
    const repo = repository();
    writePack(repo, "fresh", ["fresh-one"]);
    commit(repo, "the pack");
    releaseVersion(repo);
    const body = readFileSync(join(repo.root, "body.md"), "utf8");
    const checklist = body.slice(body.indexOf("## Before merging"));
    expect(body.indexOf(`\`${packName("fresh")}\``)).toBeLessThan(body.indexOf("## Before merging"));
    expect(checklist.match(/^- \[ \] \*\*.+$/gmu)).toHaveLength(7);
    for (const item of ["End-user testing", "Release notes", "Documentation", "`context7.json`", "`docs/internal-testing.md`", "Scrub", "npm package pages"]) {
      expect(checklist).toContain(`- [ ] **${item}.**`);
    }
  });
});

describe("release-version.mjs, each released pack's rules", () => {
  it("lists every rule of a pack never released as added", () => {
    const repo = repository();
    writePack(repo, "fresh", ["fresh-one", "fresh-two"]);
    commit(repo, "the pack");
    releaseVersion(repo);
    const changelog = readFileSync(join(repo.root, "packs/fresh/CHANGELOG.md"), "utf8");
    expect(changelog).toContain("## 0.1.1 (");
    expect(changelog).toContain("2 rules, all on by default.");
    expect(changelog).toContain("### Added");
    expect(changelog).toContain("- `fresh-one` (starts in Steer): What fresh-one wants.");
    expect(changelog).toContain("- `fresh-two` (starts in Steer): What fresh-two wants.");
    expect(changelog).not.toContain("No change to its rules");
  });

  it("lists only the rules new since a released pack's last release as added", () => {
    const repo = repository();
    writePack(repo, "tagged", ["tagged-old"]);
    commit(repo, "the pack");
    repo.git("tag", `${packName("tagged")}@0.1.0`);
    writePack(repo, "tagged", ["tagged-old", "tagged-new"]);
    commit(repo, "a new rule");
    releaseVersion(repo);
    const changelog = readFileSync(join(repo.root, "packs/tagged/CHANGELOG.md"), "utf8");
    expect(changelog).toContain("- `tagged-new` (starts in Steer): What tagged-new wants.");
    expect(changelog).not.toContain("`tagged-old`");
  });
});
