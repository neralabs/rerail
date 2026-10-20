/**
 * The input manifest of one gated tool-pack, `packs/<pack>/upstream/<tool>/gates.yaml`: which live adapter reads the
 * upstream, the pins it is read at, and the team's recorded decisions about it (its YAML comments). It is NRV's pack
 * manifest (validation/packs/<pack>.yaml) without what rerail's layout already says: the documents come from the
 * pack's conformance.json, and the rules are read from this checkout rather than a pinned rerail commit.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseDocument } from "yaml";
import { z } from "zod";

/** Every tool-pack's manifest: `packs/<pack>/upstream/<tool>/gates.yaml`. */
export const GATES_MANIFEST = "gates.yaml";

/** The live adapters a tool-pack can name, one per upstream linter family (recorders/port-readiness/adapters/). */
export const ADAPTERS = ["bandit", "ruff", "eslint", "phpcs", "tlint", "blade-parser", "phpstan", "psalm", "bladestan"] as const;

const nonEmpty = z.string().min(1);
const commit = z.string().regex(/^[0-9a-f]{40}$/u, "a full commit sha");
const githubRepository = z.string().regex(/^https:\/\/github\.com\/[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/u, "a https://github.com/<owner>/<name> URL");
const repositoryPath = z.string().regex(/^[A-Za-z0-9._-]+(?:\/[A-Za-z0-9._-]+)*$/u, "a path relative to the repository");
const npmPackage = z.string().regex(/^(?:@[a-z0-9._-]+\/)?[a-z0-9._-]+$/u, "an npm package name");
/** A directory under conformance/recorders/port-readiness/runtimes/ that pins a runtime by its package.json or pyproject.toml. */
const runtimeDirectory = z.string().regex(/^[a-z0-9._-]+$/u, "a directory under conformance/recorders/port-readiness/runtimes/");

/** A list whose items are all different. */
function unique<T extends z.ZodType>(item: T) {
  return z.array(item).refine((items) => new Set(items).size === items.length, "lists an item twice");
}

const knownGapSchema = z.strictObject({
  id: z.string().regex(/^[a-z0-9]+(?:-[a-z0-9]+)*$/u),
  /**
   * Source the engine cannot parse and independent parsers read as valid source, in the file type the gap's fixtures
   * share, occurring in each of them; every run proves the gap again with it.
   */
  probe: nonEmpty,
  /** The upstream test files (cases' `fixture`) the gap excuses, and only where the engine reported a parse failure. */
  fixtures: unique(nonEmpty).min(1),
  reason: nonEmpty,
});

/**
 * Listed engine parser gaps: source the engine cannot parse in upstream's test files. Every run proves each gap again
 * with its probe and requires independent parsers to accept the probe. A gap excuses `matches-upstream-tests`
 * mismatches only in the fixtures it names, and only where the engine reported a parse failure; a gap that does not
 * hold, or excuses nothing, fails the run.
 */
export type KnownGap = z.infer<typeof knownGapSchema>;

const realCodeGapSchema = z.strictObject({
  /** The document id of a rule of this tool-pack. */
  rule: nonEmpty,
  /** How many of NRV's goldens for the rule it differs on, engine gaps included, when the entry was written: what it stands for. */
  differing_files: z.number().int().positive(),
});

/**
 * A rule that differs from upstream on real code throughout, as NRV's real-code gate found: one entry in place of a
 * golden per differing file. The rule fails `matches-upstream-tests` until it agrees there and its goldens are imported
 * (docs/port-readiness.md, "Goldens").
 */
export type RealCodeGap = z.infer<typeof realCodeGapSchema>;

/**
 * An ESLint-family upstream: ESLint's own rules (no plugin; eslint:recommended comes from @eslint/js) or one plugin's.
 * Paths are relative to upstream's repository at the pinned commit.
 */
const eslintSchema = z.strictObject({
  /** The npm package the rules come from and the namespace their ids take (`@typescript-eslint/no-explicit-any`); null for ESLint's own rules. */
  plugin: z.strictObject({
    package: npmPackage,
    namespace: z.string().regex(/^(?:@[a-z0-9._-]+|[a-z0-9._-]+)(?:\/[a-z0-9._-]+)?$/u),
  }).nullable(),
  /**
   * The npm package (alias included) of the ESLint that upstream's tests run and the gates lint with; `eslint` when
   * omitted. facebook/react keeps eslint 7 as `eslint`, which its older TypeScript parsers load, and runs its rule
   * tests on `eslint-v9`.
   */
  eslint_package: npmPackage.optional(),
  release: z.strictObject({
    /** The release tag the published package was built from. */
    tag: nonEmpty,
    /** The directory the published package is built from; its parent holds the package's package.json. */
    path: repositoryPath,
    /** Where the runtime installs that directory, under node_modules; upstream's tests run the installed code. */
    installed: z.string().regex(/^(?:@[a-z0-9._-]+\/)?[a-z0-9._-]+(?:\/[A-Za-z0-9._-]+)*$/u),
    /** True when the package ships `path` as it is, so the installed files must equal the checkout's. */
    verbatim: z.boolean(),
    /**
     * True when upstream's tests load the plugin by package name from its sources, an ES module whose default export
     * is the plugin (`require("eslint-plugin-react-hooks").default`), and the published build is CommonJS without one:
     * the tests get the installed build as that module.
     */
    default_export: z.boolean().optional(),
    /**
     * The file at the pinned commit that declares the release's version when the package's package.json does not: the
     * version is the string an object literal in it maps the package's name to. facebook/react stamps each package's
     * version from ReactVersions.js when it builds, so packages/eslint-plugin-react-hooks/package.json at the
     * eslint-plugin-react-hooks@7.1.1 tag says 7.0.0. The package's package.json version when omitted.
     */
    version_file: repositoryPath.optional(),
  }),
  /**
   * Upstream's license file at the pinned commit (vercel/next.js keeps `license.md`); when omitted, LICENSE at the
   * repository's root in any letter case (eslint-plugin-unicorn keeps `license`).
   */
  license: repositoryPath.optional(),
  /**
   * The shareable configuration whose environment the pack is linted in, when the approved configurations are only the
   * plugin's rules (eslint-config-next for the Next.js and React packs): its file globs, parsers and globals come before
   * the approved configurations for upstream's test cases and the corpus; its plugins, rules and settings are not the
   * pack's. `runtime` pins it by package.json; `source` is where the published package is built from.
   */
  environment: z.strictObject({
    package: nonEmpty,
    version: nonEmpty,
    runtime: runtimeDirectory,
    source: z.strictObject({ repository: githubRepository, commit, path: nonEmpty }),
  }).optional(),
  /** The implementation of each rule; `{rule}` stands for the rule name, `{Rule}` for it in PascalCase (`RulesOfHooks`). */
  implementation: z.string().regex(/\{(?:rule|Rule)\}/u, "names the rule as {rule} or {Rule}"),
  /** Globs of every upstream rule test file; each RuleTester run in them names the rule it tests. */
  tests: z.array(nonEmpty).min(1),
  /**
   * Globs of the other files upstream's rule tests load as code (typescript-eslint's type-checked cases compile against
   * packages/eslint-plugin/tests/fixtures); their words are test words too.
   */
  test_fixtures: z.array(nonEmpty).optional(),
  /**
   * The checkout directory upstream's test runner runs in, when what its tests load reads the working directory
   * (@babel/eslint-parser finds facebook/react's Babel configuration from it); the runtime's directory when omitted.
   */
  test_cwd: repositoryPath.optional(),
  /**
   * The npm packages whose modules count as upstream's code where upstream's code imports them: the helper libraries
   * its rules are written with (@eslint-community/eslint-utils), resolved in the pinned runtime.
   */
  helpers: unique(npmPackage),
  /** The sparse-checkout patterns of the upstream checkout. */
  checkout: z.array(z.string().regex(/^\//u, "starts with /")).min(1),
  /**
   * Checkout directories of upstream's test fixture parsers that return the tree another parser built for each case's
   * own code (ESLint's tests/fixtures/parsers/typescript-parsers/), mapped to the file type of that code. Any other
   * parser upstream's tests load from the checkout returns a fixed tree whatever the code, and its cases are skipped.
   */
  fixture_parsers: z.record(z.string().regex(/^[A-Za-z0-9._-]+(?:\/[A-Za-z0-9._-]+)*\/$/u), z.string().regex(/^\.[a-z]+$/u)).optional(),
  /**
   * The npm packages (aliases included) of the parsers upstream's tests lint with that declare no `meta.name`, mapped
   * to the language their trees are of. A case linted by another parser without a name is skipped.
   */
  parsers: z.record(npmPackage, z.enum(["javascript", "typescript"])).optional(),
  /**
   * Checkout modules of upstream's own rule testers besides ESLint's and typescript-eslint's RuleTester
   * (eslint-plugin-unicorn's test/utils/snapshot-rule-tester.js). Each default-exports a class whose
   * run(ruleName, rule, {valid, invalid}) registers one test per case, valid cases first, and each test lints its case
   * with Linter.verify, the rule configured as `rule-to-test/<ruleName>`.
   */
  case_testers: unique(z.string().regex(/^[A-Za-z0-9._-]+(?:\/[A-Za-z0-9._-]+)*\.[cm]?js$/u)).optional(),
});

/** Which ESLint rules a tool-pack ports and where upstream keeps them; required for the eslint adapter. */
export type EslintUpstream = z.infer<typeof eslintSchema>;

export const gatesManifestSchema = z.strictObject({
  /** The live adapter that reads this upstream (recorders/port-readiness/adapters/<adapter>.ts). */
  adapter: z.enum(ADAPTERS),
  rerail: z.strictObject({
    /** The pack's id: the directory under packs/ this manifest sits in. */
    pack: z.string().regex(/^[a-z0-9-]+$/u),
    /** The upstream project's directory under packs/<pack>/licenses/, which keeps its LICENSE and UPSTREAM.md. */
    project: z.string().regex(/^[A-Za-z0-9._-]+$/u),
    /**
     * The name of upstream's licence file, at the root of upstream's repository and kept under licenses/<project>/,
     * when it is not LICENSE (PHP_CodeSniffer's is `licence.txt`).
     */
    license: z.string().regex(/^[A-Za-z0-9._-]+$/u).optional(),
    /**
     * The file types rerail's ports of this tool cover, when the pack deliberately covers fewer than upstream lints:
     * `loads`'s in-scope check then asks for these alone. Each must be a type upstream lints (the `in-scope-narrowing`
     * tool-pack check). Absent, the ports must cover every type upstream lints.
     */
    in_scope: unique(z.string().regex(/^\.[a-z]+$/u)).min(1).optional(),
  }),
  upstream: z.strictObject({
    name: nonEmpty,
    repository: githubRepository,
    commit,
    /** The release the pins name; conformance.json's tools[].version and the recording's pins must agree with it. */
    version: nonEmpty,
    license: nonEmpty,
    /**
     * The upstream configurations that count as recommended, in order. A rule starts on when one turns it on
     * (`upstream-status`), and `matches-upstream-tests` checks it with the options of the first that does, on
     * upstream's tests and on its goldens.
     */
    recommended_configs: unique(nonEmpty).min(1),
    /** The directory under conformance/recorders/port-readiness/runtimes/ that pins the runtime upstream is run with. */
    runtime: runtimeDirectory,
  }),
  /** Recorded evidence that the upstream is public and popular: tool-pack registration data, not a per-rule label. */
  popularity: z.strictObject({
    observed_on: z.iso.date(),
    evidence: z.array(nonEmpty).min(1),
  }),
  known_gaps: z.array(knownGapSchema),
  real_code_gaps: z.array(realCodeGapSchema).default([]),
  eslint: eslintSchema.optional(),
}).superRefine((manifest, context) => {
  if ((manifest.adapter === "eslint") !== (manifest.eslint !== undefined)) {
    context.addIssue({ code: "custom", path: ["eslint"], message: "the eslint adapter needs the eslint block, and no other adapter takes one" });
  }
  const ids = manifest.known_gaps.map((gap) => gap.id);
  const repeated = ids.find((id, index) => ids.indexOf(id) !== index);
  if (repeated !== undefined) context.addIssue({ code: "custom", path: ["known_gaps"], message: `lists the known gap ${repeated} twice` });
  const rules = manifest.real_code_gaps.map((gap) => gap.rule);
  const twice = rules.find((rule, index) => rules.indexOf(rule) !== index);
  if (twice !== undefined) context.addIssue({ code: "custom", path: ["real_code_gaps"], message: `lists ${twice} twice` });
});

export type GatesManifest = z.infer<typeof gatesManifestSchema>;

/** The runtimes each adapter runs besides upstream's own (gates.yaml `upstream.runtime`). */
const EXTRA_RUNTIMES: Record<GatesManifest["adapter"], string[]> = { bandit: ["python"], ruff: ["python"], eslint: ["parsers"], phpcs: ["php"], tlint: ["blade", "php"], "blade-parser": ["php"], phpstan: ["php"], psalm: ["php"], bladestan: ["blade", "php"] };

/**
 * Every runtime the adapter installs to record a tool-pack, sorted: upstream's own, the adapter's, and the environment
 * the pack is linted in when gates.yaml names one (eslint-config-next's parsers and globals shape upstream's answers as
 * much as the plugin does). The recorder pins each of them in the lock, and the gates' pins-agree check holds a lock to
 * exactly these, so a runtime gates.yaml names and the lock doesn't makes the recording stale, not unchecked.
 */
export function recordedRuntimes(manifest: GatesManifest): string[] {
  const environment = manifest.eslint?.environment?.runtime;
  const runtimes = [manifest.upstream.runtime, ...EXTRA_RUNTIMES[manifest.adapter], ...(environment === undefined ? [] : [environment])];
  return [...new Set(runtimes)].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/** Strict YAML: unique keys, no anchors or aliases, and a mapping at the top level. */
export function parseYamlMapping(text: string, label: string): Record<string, unknown> {
  const document = parseDocument(text, { uniqueKeys: true });
  if (document.errors.length > 0) throw new Error(`${label}: ${document.errors.map((error) => error.message).join("; ")}`);
  const value: unknown = document.toJS({ maxAliasCount: 0 });
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label}: must be a YAML mapping`);
  return value as Record<string, unknown>;
}

/** A gates.yaml's text, checked against the schema; `label` names it in errors. */
export function parseGatesManifest(text: string, label: string): GatesManifest {
  const parsed = gatesManifestSchema.safeParse(parseYamlMapping(text, label));
  if (!parsed.success) {
    throw new Error(`${label}: ${parsed.error.issues.map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  }
  return parsed.data;
}

/** The tool-pack directory a gates.yaml and its recordings sit in: `packs/<pack>/upstream/<tool>`, under `root`. */
export function toolPackDirectory(root: string, pack: string, tool: string): string {
  return join(root, "packs", pack, "upstream", tool);
}

/** Read `packs/<pack>/upstream/<tool>/gates.yaml` under `root`; its `rerail.pack` must be the pack it sits in. */
export function readGatesManifest(root: string, pack: string, tool: string): GatesManifest & { id: string } {
  const label = `packs/${pack}/upstream/${tool}/${GATES_MANIFEST}`;
  const manifest = parseGatesManifest(readFileSync(join(toolPackDirectory(root, pack, tool), GATES_MANIFEST), "utf8"), label);
  if (manifest.rerail.pack !== pack) throw new Error(`${label}: rerail.pack is ${manifest.rerail.pack}, but the manifest sits in packs/${pack}/`);
  return { ...manifest, id: `${pack}/${tool}` };
}
