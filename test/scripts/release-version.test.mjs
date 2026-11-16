// release-version.mjs run for real, in a throwaway git repository holding a copy of the release scripts: it reads its
// root from its own path, compares rules across git history, and runs at the top level, so nothing short of this
// reaches that comparison. `version:packs` is a stub that bumps every pack, as `changeset version` does for the packs a
// changeset names.
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { linkFolder } from "../support/platform/links.ts";
import { removeFolderSync } from "../support/platform/remove-folder.ts";

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const BUMP = `import { readdirSync, readFileSync, writeFileSync } from "node:fs";
for (const id of readdirSync("packs")) {
  const path = \`packs/\${id}/package.json\`;
  writeFileSync(path, JSON.stringify({ ...JSON.parse(readFileSync(path, "utf8")), version: "0.1.1" }, null, 2) + "\\n");
}
`;

const directories = [];
afterEach(() => {
  for (const directory of directories.splice(0)) removeFolderSync(directory);
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
  linkFolder(join(REPOSITORY, "node_modules"), join(root, "node_modules"));
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

/**
 * Whether release-version.mjs can run here: it starts `npm` as a program, and Windows has only `npm.cmd`, which starts
 * through a shell. Releases are cut on Linux in CI.
 */
const NPM_RUNS = process.platform !== "win32";

function releaseVersion({ root }) {
  execFileSync("node", ["scripts/release/release-version.mjs", join(root, "body.md")], { cwd: root, stdio: "pipe" });
}

describe.runIf(NPM_RUNS)("release-version.mjs, the pull request's description", () => {
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

describe.runIf(NPM_RUNS)("release-version.mjs, each released pack's rules", () => {
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

describe.runIf(NPM_RUNS)("release-version.mjs, a released pack's core range", () => {
  const CORE = "@neralabs/rerail";
  const java = { mode: "steer", description: "A Java rule.", clauses: [{ forbid: { ast: { language: "java" } } }] };
  const python = { mode: "steer", description: "A Python rule.", clauses: [{ forbid: { ast: { language: "python" } } }] };

  /** Writes pack `id` at 0.1.0 with the core range `range`, its rules `{ id: document }`, and `heldOff` naming the ones it holds off. */
  function writeRangedPack({ write }, id, range, rules, heldOff = []) {
    const manifest = { name: packName(id), version: "0.1.0", peerDependencies: { [CORE]: range }, peerDependenciesMeta: { [CORE]: { optional: true } } };
    write(`packs/${id}/package.json`, `${JSON.stringify(manifest, null, 2)}\n`);
    const entries = Object.keys(rules).map((rule) => ({ id: rule, document: `rules/${rule}.json`, ...(heldOff.includes(rule) ? { on: false, heldOff: "Its steer does harm." } : {}) }));
    write(`packs/${id}/pack.json`, `${JSON.stringify({ rules: entries }, null, 2)}\n`);
    for (const [rule, document] of Object.entries(rules)) write(`packs/${id}/rules/${rule}.json`, `${JSON.stringify(document)}\n`);
  }

  /** A lockfile recording each of `ids` as a workspace with the core range `range`. */
  function writeLock({ write }, ids, range) {
    const packages = { "": { name: "core", version: "1.0.0" } };
    for (const id of ids) packages[`packs/${id}`] = { name: packName(id), version: "0.1.0", peerDependencies: { [CORE]: range }, peerDependenciesMeta: { [CORE]: { optional: true } } };
    write("package-lock.json", `${JSON.stringify({ name: "core", version: "1.0.0", lockfileVersion: 3, packages }, null, 2)}\n`);
  }

  const rangeOf = ({ root }, id) => JSON.parse(readFileSync(join(root, `packs/${id}/package.json`), "utf8")).peerDependencies[CORE];
  const lockedRangeOf = ({ root }, id) => JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8")).packages[`packs/${id}`].peerDependencies[CORE];

  it("raises it for a pack holding a rule off and for one with a rule in a language added since, in package.json and the lockfile, and leaves the others", () => {
    const repo = repository();
    writeRangedPack(repo, "held", ">=0.1.0-0", { "held-one": python, "held-two": python }, ["held-two"]);
    writeRangedPack(repo, "javaish", ">=0.1.0-0", { "javaish-one": java });
    writeRangedPack(repo, "plain", ">=0.1.0-0", { "plain-one": python });
    writeLock(repo, ["held", "javaish", "plain"], ">=0.1.0-0");
    commit(repo, "the packs");
    const plainBefore = readFileSync(join(repo.root, "packs/plain/package.json"), "utf8");
    releaseVersion(repo);
    expect([rangeOf(repo, "held"), lockedRangeOf(repo, "held")]).toEqual([">=0.1.5-0", ">=0.1.5-0"]);
    expect([rangeOf(repo, "javaish"), lockedRangeOf(repo, "javaish")]).toEqual([">=0.1.5-0", ">=0.1.5-0"]);
    expect([rangeOf(repo, "plain"), lockedRangeOf(repo, "plain")]).toEqual([">=0.1.0-0", ">=0.1.0-0"]);
    // Only the version the stub bumped differs in the pack no older core refuses.
    expect(readFileSync(join(repo.root, "packs/plain/package.json"), "utf8")).toBe(plainBefore.replace('"0.1.0"', '"0.1.1"'));
    const body = readFileSync(join(repo.root, "body.md"), "utf8");
    expect(body).toContain(`- \`${packName("held")}\`: \`>=0.1.0-0\` → \`>=0.1.5-0\``);
    expect(body).toContain(`- \`${packName("javaish")}\`: \`>=0.1.0-0\` → \`>=0.1.5-0\``);
    expect(body).not.toContain(`- \`${packName("plain")}\`: `);
    expect(body).not.toContain("Not released by this pull request");
  });

  it("changes nothing but the range in the package.json it raises", () => {
    const repo = repository();
    writeRangedPack(repo, "javaish", ">=0.1.0-0", { "javaish-one": java });
    commit(repo, "the pack");
    const before = readFileSync(join(repo.root, "packs/javaish/package.json"), "utf8");
    releaseVersion(repo);
    expect(readFileSync(join(repo.root, "packs/javaish/package.json"), "utf8")).toBe(before.replace('"0.1.0"', '"0.1.1"').replace('">=0.1.0-0"', '">=0.1.5-0"'));
  });

  it("leaves a range alone while core is older than the release that first loads the pack", () => {
    const repo = repository();
    repo.write("package.json", `${JSON.stringify({ name: "core", version: "0.1.4", type: "module", scripts: { "version:packs": "node bump.mjs" } }, null, 2)}\n`);
    writeRangedPack(repo, "javaish", ">=0.1.0-0", { "javaish-one": java });
    writeLock(repo, ["javaish"], ">=0.1.0-0");
    commit(repo, "the pack");
    releaseVersion(repo);
    expect([rangeOf(repo, "javaish"), lockedRangeOf(repo, "javaish")]).toEqual([">=0.1.0-0", ">=0.1.0-0"]);
    expect(readFileSync(join(repo.root, "body.md"), "utf8")).not.toContain("Core range raised");
  });

  it("leaves a range that already leaves older cores out", () => {
    const repo = repository();
    writeRangedPack(repo, "javaish", ">=0.1.5-0", { "javaish-one": java });
    commit(repo, "the pack");
    releaseVersion(repo);
    expect(rangeOf(repo, "javaish")).toBe(">=0.1.5-0");
    expect(readFileSync(join(repo.root, "body.md"), "utf8")).not.toContain("Core range raised");
  });

  it("warns of a pack that needs it and is not released, and does not edit it", () => {
    const repo = repository();
    // The stub bumps only `moving`, as `changeset version` bumps only the packs a changeset names.
    repo.write("bump.mjs", BUMP.replace('for (const id of readdirSync("packs")) {', 'for (const id of ["moving"]) {'));
    writeRangedPack(repo, "moving", ">=0.1.0-0", { "moving-one": python });
    writeRangedPack(repo, "idle", ">=0.1.0-0", { "idle-one": java });
    commit(repo, "the packs");
    const idleBefore = readFileSync(join(repo.root, "packs/idle/package.json"), "utf8");
    releaseVersion(repo);
    expect(readFileSync(join(repo.root, "packs/idle/package.json"), "utf8")).toBe(idleBefore);
    const body = readFileSync(join(repo.root, "body.md"), "utf8");
    expect(body).toContain(`Not released by this pull request, and an older Rerail cannot load it: \`${packName("idle")}\` (\`>=0.1.0-0\`, needs \`>=0.1.5-0\`).`);
    expect(body).not.toContain("Core range raised");
  });
});
