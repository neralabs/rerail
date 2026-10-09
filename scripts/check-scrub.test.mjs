import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, test } from "vitest";

import { checkScrub } from "./check-scrub.mjs";

const SCRIPT = fileURLToPath(new URL("./check-scrub.mjs", import.meta.url));
const homes = [];

afterEach(() => {
  for (const home of homes.splice(0)) rmSync(home, { recursive: true, force: true });
});

/** Git isolated from this machine's configuration and from any repository the tests run inside. */
function gitEnvironment(home) {
  const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("GIT_")));
  return {
    ...environment,
    GIT_CONFIG_GLOBAL: join(home, "gitconfig"),
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "Rerail Test",
    GIT_AUTHOR_EMAIL: "test@example.invalid",
    GIT_COMMITTER_NAME: "Rerail Test",
    GIT_COMMITTER_EMAIL: "test@example.invalid",
  };
}

/** Writes `files` under `root`: a string as is, null deletes, anything else as JSON. */
function write(root, files) {
  for (const [path, content] of Object.entries(files)) {
    const target = join(root, path);
    if (content === null) {
      rmSync(target, { recursive: true, force: true });
      continue;
    }
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, typeof content === "string" ? content : `${JSON.stringify(content, null, 2)}\n`);
  }
}

const document = id => `{"format":"rerail.policy/v1","id":"${id}","clauses":[]}\n`;
const port = (id, tool, policy = `rules/${id}.json`) => ({ id, upstream: { tool, rule: id.toUpperCase() }, policy, capabilities: ["syntax"], match: "line" });
const packJson = (id, rules) => ({ format: "rerail.rule-pack/v1", id, rules: rules.map(rule => ({ id: `${id}:${rule}`, document: `rules/${rule}.json` })) });

/** conformance/checks.ts at the base, with the sub-check ids the base had. */
const checksTs = ids => `export const SUB_CHECKS = [\n${ids.map(id => `  { id: "${id}", check: "${id.split(":")[0]}" },\n`).join("")}] as const;\n`;

/**
 * The base: alpha (ruff with source facts, a scrub list, one divergence), gamma (source facts, no scrub list), beta
 * (a tool without source facts) and golden (TSLint's golden manifest and audit).
 */
const BASE = {
  "conformance/checks.ts": checksTs(["rules", "ports:points", "ports:span", "provenance:recommended"]),
  "packs/alpha/pack.json": packJson("alpha", ["a", "b"]),
  "packs/alpha/conformance.json": { pack: "alpha", tools: [{ name: "ruff", version: "0.16.9", recommended: ["default"] }], rules: [port("a", "ruff"), port("b", "ruff")] },
  "packs/alpha/divergences.json": [{ rule: "a", source: "fixtures", path: "a.py", line: 3, kind: "upstream-wrong", evidence: "Ruff 0.16.9 reports the wrong line here." }],
  "packs/alpha/scrub.json": [{ rule: "alpha:a", check: "ports:span" }],
  "packs/alpha/rules/a.json": document("alpha:a"),
  "packs/alpha/rules/b.json": document("alpha:b"),
  "packs/gamma/pack.json": packJson("gamma", ["e"]),
  "packs/gamma/conformance.json": { pack: "gamma", tools: [{ name: "bandit", version: "1.8.6", recommended: ["default"] }], rules: [port("e", "bandit")] },
  "packs/gamma/rules/e.json": document("gamma:e"),
  "packs/beta/pack.json": packJson("beta", ["c"]),
  "packs/beta/conformance.json": { pack: "beta", tools: [{ name: "phpstan", version: "2.2.16" }], rules: [port("c", "phpstan")] },
  "packs/beta/divergences.json": [],
  "packs/beta/rules/c.json": document("beta:c"),
  "packs/golden/pack.json": packJson("golden", ["d"]),
  "packs/golden/conformance.json": { format: "rerail-tslint-golden/v1", rules: { d: { verdict: "PASS" } } },
  "packs/golden/divergences.json": { format: "rerail-tslint-audit/v1", rules: {} },
  "packs/golden/rules/d.json": document("golden:d"),
};

/** A repository whose one commit is `base` (BASE with `changes`), with `head` applied to its working tree. */
function repository(head = {}, changes = {}) {
  const home = mkdtempSync(join(tmpdir(), "check-scrub-"));
  homes.push(home);
  writeFileSync(join(home, "gitconfig"), "");
  const root = join(home, "repository");
  mkdirSync(root);
  const git = args => execFileSync("git", args, { cwd: root, env: gitEnvironment(home), stdio: ["ignore", "pipe", "pipe"] });
  git(["init", "-q", "-b", "main"]);
  write(root, { ...BASE, ...changes });
  git(["add", "-A"]);
  git(["commit", "-q", "-m", "base"]);
  write(root, head);
  return root;
}

const problems = (head, changes) => checkScrub({ root: repository(head, changes), base: "HEAD" }).problems;

/** Asserts exactly one problem, matching `pattern`. */
function refusedOnce(head, pattern, changes) {
  const found = problems(head, changes);
  assert.equal(found.length, 1, found.join("\n"));
  assert.match(found[0], pattern);
}

test("an unchanged tree passes", () => {
  assert.deepEqual(problems(), []);
});

test("scrub.json may shrink", () => {
  assert.deepEqual(problems({ "packs/alpha/scrub.json": [] }), []);
});

test("an existing list refuses an entry for a sub-check the base had", () => {
  refusedOnce(
    { "packs/alpha/scrub.json": [...BASE["packs/alpha/scrub.json"], { rule: "alpha:b", check: "ports:span" }] },
    /^packs\/alpha\/scrub\.json: new entry \(alpha:b, ports:span\): scrub\.json may only shrink, and ports:span was a sub-check at [0-9a-f]{12}$/u,
  );
});

test("a new sub-check is admitted on a byte-identical document, and refused on an edited one", () => {
  const added = { "packs/alpha/scrub.json": [...BASE["packs/alpha/scrub.json"], { rule: "alpha:b", check: "generality:names" }] };
  assert.deepEqual(problems(added), []);
  refusedOnce({ ...added, "packs/alpha/rules/b.json": document("alpha:b").replace("[]", "[ ]") }, /new entry \(alpha:b, generality:names\): rules\/b\.json differs from [0-9a-f]{12}'s/u);
});

test("a base without conformance/checks.ts had no sub-checks, so every id is new", () => {
  const added = { "packs/alpha/scrub.json": [...BASE["packs/alpha/scrub.json"], { rule: "alpha:b", check: "ports:span" }] };
  assert.deepEqual(problems(added, { "conformance/checks.ts": null }), []);
});

test("a pack with no scrub.json at the base starts no list for a sub-check the base had", () => {
  refusedOnce({ "packs/gamma/scrub.json": [{ rule: "gamma:e", check: "ports:span" }] }, /new entry \(gamma:e, ports:span\): scrub\.json may only shrink, and ports:span was a sub-check at [0-9a-f]{12}$/u);
  assert.deepEqual(problems({ "packs/gamma/scrub.json": [{ rule: "gamma:e", check: "generality:names" }] }), []);
});

test("a new sub-check is refused on a new rule's document", () => {
  refusedOnce(
    {
      "packs/gamma/pack.json": packJson("gamma", ["e", "f"]),
      "packs/gamma/conformance.json": { ...BASE["packs/gamma/conformance.json"], rules: [port("e", "bandit"), port("f", "bandit")] },
      "packs/gamma/rules/f.json": document("gamma:f"),
      "packs/gamma/scrub.json": [{ rule: "gamma:f", check: "generality:names" }],
    },
    /new entry \(gamma:f, generality:names\): [0-9a-f]{12} does not list the rule: a new rule must pass every check$/u,
  );
});

test("a one-byte edit to a document refuses its new entry", () => {
  refusedOnce(
    { "packs/gamma/rules/e.json": document("gamma:e").replace("v1", "v2"), "packs/gamma/scrub.json": [{ rule: "gamma:e", check: "generality:names" }] },
    /new entry \(gamma:e, generality:names\): rules\/e\.json differs from [0-9a-f]{12}'s; an entry needs the document byte-identical to the base's$/u,
  );
});

test("a renamed document refuses its new entry, although its bytes are unchanged", () => {
  refusedOnce(
    {
      "packs/gamma/pack.json": { ...packJson("gamma", []), rules: [{ id: "gamma:e", document: "rules/e-renamed.json" }] },
      "packs/gamma/conformance.json": { ...BASE["packs/gamma/conformance.json"], rules: [port("e", "bandit", "rules/e-renamed.json")] },
      "packs/gamma/rules/e.json": null,
      "packs/gamma/rules/e-renamed.json": document("gamma:e"),
      "packs/gamma/scrub.json": [{ rule: "gamma:e", check: "generality:names" }],
    },
    /new entry \(gamma:e, generality:names\): its document moved from rules\/e\.json to rules\/e-renamed\.json/u,
  );
});

test("an entry the base already has survives a document edit", () => {
  assert.deepEqual(problems({ "packs/alpha/rules/a.json": document("alpha:a").replace("[]", "[ ]") }), []);
});

test("deleting a scrub.json shrinks it to nothing, and opens no new list", () => {
  assert.deepEqual(problems({ "packs/alpha/scrub.json": null }), []);
  assert.deepEqual(problems({ "packs/alpha": null }), []);
  refusedOnce({ "packs/alpha/scrub.json": [{ rule: "alpha:b", check: "ports:span" }] }, /new entry \(alpha:b, ports:span\): scrub\.json may only shrink/u, { "packs/alpha/scrub.json": null });
});

test("divergences.json may only shrink", () => {
  const [known] = BASE["packs/alpha/divergences.json"];
  assert.deepEqual(problems({ "packs/alpha/divergences.json": [] }), []);
  assert.deepEqual(problems({ "packs/alpha/divergences.json": [{ ...known, evidence: "Ruff 0.16.9 puts this finding on the decorator line." }] }), []);
  refusedOnce(
    { "packs/alpha/divergences.json": [known, { ...known, path: "b.py", kind: "rerail-unknown" }] },
    /^packs\/alpha\/divergences\.json: new divergence \(a, rerail-unknown, fixtures b\.py:3\); divergences\.json may only shrink/u,
  );
  refusedOnce({ "packs/alpha/divergences.json": [{ ...known, line: 4 }] }, /new divergence \(a, upstream-wrong, fixtures a\.py:4\)/u);
  refusedOnce({ "packs/alpha/divergences.json": [{ ...known, source: undefined }] }, /new divergence \(a, upstream-wrong, every source a\.py:3\)/u);
  refusedOnce({ "packs/beta/divergences.json": [{ ...known, rule: "c" }] }, /^packs\/beta\/divergences\.json: new divergence \(c,/u);
  refusedOnce({ "packs/gamma/divergences.json": [{ ...known, rule: "e" }] }, /^packs\/gamma\/divergences\.json: new divergence \(e,/u);
});

test("another harness's divergences.json is not read", () => {
  assert.deepEqual(problems({ "packs/golden/divergences.json": { format: "rerail-tslint-audit/v1", rules: { d: { class: "genuine", note: "Matches TSLint." } } } }), []);
});

test("a new rule is refused where checks 4 and 7 to 9 cannot judge it", () => {
  refusedOnce(
    {
      "packs/beta/pack.json": packJson("beta", ["c", "x"]),
      "packs/beta/conformance.json": { ...BASE["packs/beta/conformance.json"], rules: [port("c", "phpstan"), port("x", "phpstan")] },
      "packs/beta/rules/x.json": document("beta:x"),
    },
    /^packs\/beta\/pack\.json: new rule beta:x: no source facts recorded for phpstan \(conformance\.json tools\[\]\.recommended\); a new rule must face every check$/u,
  );
  refusedOnce(
    { "packs/golden/pack.json": packJson("golden", ["d", "z"]), "packs/golden/rules/z.json": document("golden:z") },
    /new rule golden:z: conformance\.json is rerail-tslint-golden\/v1, another harness's, so checks 4 and 7 to 9 do not judge its rules/u,
  );
});

test("a new rule without a port is admitted in any pack but a golden manifest's: its examples prove it", () => {
  assert.deepEqual(problems({ "packs/beta/pack.json": packJson("beta", ["c", "y"]), "packs/beta/rules/y.json": document("beta:y") }), []);
  assert.deepEqual(problems({ "packs/delta/pack.json": packJson("delta", ["w"]), "packs/delta/rules/w.json": document("delta:w") }), []);
});

test("an existing port that loses its port entry is refused", () => {
  const lost = /^packs\/(alpha|beta|gamma)\/pack\.json: (alpha:b|beta:c|gamma:e) has a port entry in conformance\.json at [0-9a-f]{12} and none now; an existing port keeps facing checks 4 and 7 to 9$/u;
  refusedOnce({ "packs/alpha/conformance.json": { ...BASE["packs/alpha/conformance.json"], rules: [port("a", "ruff")] } }, lost);
  refusedOnce({ "packs/beta/conformance.json": { ...BASE["packs/beta/conformance.json"], rules: [] } }, lost);
  refusedOnce({ "packs/gamma/conformance.json": null }, lost);
});

test("an existing rule proven by examples that becomes a port its tool's facts cannot judge is refused", () => {
  const changes = { "packs/alpha/pack.json": packJson("alpha", ["a", "b", "h"]), "packs/alpha/rules/h.json": document("alpha:h") };
  refusedOnce(
    {
      "packs/alpha/conformance.json": {
        ...BASE["packs/alpha/conformance.json"],
        tools: [...BASE["packs/alpha/conformance.json"].tools, { name: "flake8", version: "7.0.0" }],
        rules: [port("a", "ruff"), port("b", "ruff"), port("h", "flake8")],
      },
    },
    /^packs\/alpha\/pack\.json: alpha:h becomes a port: no source facts recorded for flake8 \(conformance\.json tools\[\]\.recommended\); a new port must face every check$/u,
    changes,
  );
  assert.deepEqual(problems({ "packs/alpha/conformance.json": { ...BASE["packs/alpha/conformance.json"], rules: [port("a", "ruff"), port("b", "ruff"), port("h", "ruff")] } }, changes), []);
});

test("a new rule is admitted where every check judges it", () => {
  assert.deepEqual(
    problems({
      "packs/alpha/pack.json": packJson("alpha", ["a", "b", "g", "h"]),
      "packs/alpha/conformance.json": { ...BASE["packs/alpha/conformance.json"], rules: [port("a", "ruff"), port("b", "ruff"), port("g", "ruff")] },
      "packs/alpha/rules/g.json": document("alpha:g"),
      "packs/alpha/rules/h.json": document("alpha:h"),
    }),
    [],
  );
});

test("removing recommended, or moving a port to a tool without it, is refused", () => {
  const found = problems({ "packs/alpha/conformance.json": { ...BASE["packs/alpha/conformance.json"], tools: [{ name: "ruff", version: "0.16.9" }] } });
  assert.deepEqual(
    found.map(problem => problem.replace(/[0-9a-f]{12}/gu, "<base>")),
    [
      "packs/alpha/pack.json: alpha:a was judged by checks 4 and 7 to 9 at <base>, and no longer is: no source facts recorded for ruff (conformance.json tools[].recommended)",
      "packs/alpha/pack.json: alpha:b was judged by checks 4 and 7 to 9 at <base>, and no longer is: no source facts recorded for ruff (conformance.json tools[].recommended)",
      "packs/alpha/conformance.json: ruff no longer declares source facts (tools[].recommended), which <base> records",
    ],
  );
  refusedOnce(
    {
      "packs/alpha/conformance.json": {
        ...BASE["packs/alpha/conformance.json"],
        tools: [...BASE["packs/alpha/conformance.json"].tools, { name: "flake8", version: "7.0.0" }],
        rules: [port("a", "ruff"), port("b", "flake8")],
      },
    },
    /^packs\/alpha\/pack\.json: alpha:b was judged by checks 4 and 7 to 9 at [0-9a-f]{12}, and no longer is: no source facts recorded for flake8/u,
  );
});

test("a scrub.json that does not parse is a problem", () => {
  refusedOnce({ "packs/alpha/scrub.json": "[{" }, /^packs\/alpha\/scrub\.json: does not parse in the working tree: /u);
});

test("the command exits 0 when nothing new is excused, 1 with every problem, and 2 without a base", () => {
  const root = repository({ "packs/alpha/scrub.json": [...BASE["packs/alpha/scrub.json"], { rule: "alpha:b", check: "ports:span" }], "packs/beta/divergences.json": [{ rule: "c", path: "c.php", line: 1, kind: "rerail-unknown", evidence: "Not yet known why phpstan reports this." }] });
  const run = argv => spawnSync(process.execPath, [SCRIPT, ...argv], { cwd: join(root, "packs"), env: gitEnvironment(dirname(root)), encoding: "utf8" });
  const refused = run(["HEAD"]);
  assert.equal(refused.status, 1, refused.stderr);
  assert.match(refused.stderr, /^check-scrub: 2 problem\(s\) against [0-9a-f]{12}:\n {2}packs\/alpha\/scrub\.json: .*\n {2}packs\/beta\/divergences\.json: .*\n$/u);
  write(root, { "packs/alpha/scrub.json": BASE["packs/alpha/scrub.json"], "packs/beta/divergences.json": [] });
  const passed = run(["HEAD"]);
  assert.equal(passed.status, 0, passed.stderr);
  assert.match(passed.stdout, /^check-scrub: 4 pack\(s\) excuse nothing new against [0-9a-f]{12}\n$/u);
  const unknown = run(["no-such-base"]);
  assert.equal(unknown.status, 2);
  assert.match(unknown.stderr, /no-such-base is not a commit in .*; fetch the base first/u);
  assert.equal(run([]).status, 2);
});
