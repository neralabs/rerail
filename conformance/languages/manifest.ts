/**
 * The input manifest of one gated tool-pack, `packs/<pack>/upstream/<tool>/gates.yaml`: the upstream's linter family,
 * the pins its test data is derived at, and the team's recorded decisions about it (its YAML comments). It is NRV's pack
 * manifest (validation/packs/<pack>.yaml) without what rerail's layout already says: the documents come from the
 * pack's conformance.json, and the rules are read from this checkout rather than a pinned rerail commit.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parseDocument } from "yaml";
import { z } from "zod";

/** Every tool-pack's manifest: `packs/<pack>/upstream/<tool>/gates.yaml`. */
export const GATES_MANIFEST = "gates.yaml";

/** The upstream linter families a tool-pack can name: which registry publishes upstream's releases. */
export const ADAPTERS = ["bandit", "ruff", "eslint", "phpcs", "tlint", "blade-parser", "phpstan", "psalm", "bladestan", "sqlfluff", "clippy", "lizard"] as const;

const nonEmpty = z.string().min(1);
const commit = z.string().regex(/^[0-9a-f]{40}$/u, "a full commit sha");
const githubRepository = z.string().regex(/^https:\/\/github\.com\/[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/u, "a https://github.com/<owner>/<name> URL");
/** A directory under scripts/pack-data/runtimes/ that pins a runtime by its manifest. */
const runtimeDirectory = z.string().regex(/^[a-z0-9._-]+$/u, "a directory under scripts/pack-data/runtimes/");

/** A list whose items are all different. */
function unique<T extends z.ZodType>(item: T) {
  return z.array(item).refine((items) => new Set(items).size === items.length, "lists an item twice");
}

/** An ESLint-family upstream: ESLint's own rules or one plugin's, and the release its npm package was built from. */
const eslintSchema = z.strictObject({
  release: z.strictObject({
    /** The release tag the published package was built from. */
    tag: nonEmpty,
    /** The npm package (or its path under node_modules) the runtime installs, whose releases upstream publishes. */
    installed: z.string().regex(/^(?:@[a-z0-9._-]+\/)?[a-z0-9._-]+(?:\/[A-Za-z0-9._-]+)*$/u),
  }),
});

/** A file type: its extension, or a dialect's longer suffix (`.blade.php`). */
const fileType = z.string().regex(/^(?:\.[a-z0-9]+)+$/u, "a file type such as .py or .blade.php");

/**
 * What upstream says of one of its rules at the pins, declared and reviewed at each bump: `deprecated` is upstream's
 * deprecation message, or null; `recommended` is the first of `upstream.recommended_configs` that turns the rule on,
 * with its options, or null. `rename_exempt`, when given, says why the rule's meaning is the very names the rename
 * variants change (a typo check against fixed export names, say): holds-under-rename then does not fail the rule when
 * the renaming takes every finding, though an FP, FN or engine failure on its variants still fails it.
 */
const upstreamRuleFactsSchema = z.strictObject({
  deprecated: z.string().min(1).nullable(),
  recommended: z.strictObject({ config: nonEmpty, options: z.array(z.json()) }).nullable(),
  rename_exempt: nonEmpty.optional(),
});
export type UpstreamRuleFacts = z.infer<typeof upstreamRuleFactsSchema>;

/** The upstream index files a watch's `code` can name, from which the nightly finds each ported rule's code file. */
export const WATCH_INDEXES = ["clippy-declared-lints", "ruff-codes"] as const;
export type WatchIndex = (typeof WATCH_INDEXES)[number];

/**
 * A path or glob relative to upstream's repository root (`*` stays within a segment, `**` crosses them, as `pathGlob`
 * reads them), with no `.` or `..` segment; `{rule}` stands for each ported rule where `withRule` allows it.
 */
function watchPattern(withRule: boolean) {
  return z.string().regex(/^(?!\/)(?!.*(?:^|\/)\.{1,2}(?:\/|$))[^\s\\]+$/u, "a path relative to upstream's repository root").refine(
    (pattern) => withRule || !pattern.includes("{rule}"),
    "takes no {rule}",
  ).refine(
    (pattern) => pattern.replaceAll("{rule}", "").match(/[{}]/u) === null,
    "holds a brace other than {rule}",
  );
}

/** A JSON data file beside the generator, `{ "rules": { "<ported rule>": ["<upstream path>", …] } }`. */
const dataFile = z.string().regex(/^[A-Za-z0-9._-]+\.json$/u, "a JSON file beside the generator");

/**
 * A data file whose paths are watched; `with` also watches each path with its extension replaced by each of these
 * (`tests/ui/x.rs` with `.stderr` is `tests/ui/x.stderr`, the expected output the generator reads beside the test).
 */
const watchFileSchema = z.strictObject({
  file: dataFile,
  with: unique(z.string().regex(/^(?:\.[A-Za-z0-9_-]+)+$/u, "an extension such as .stderr")).min(1).optional(),
});

/** A data file listing PHPStan-style tests: the files each names as `__DIR__ . '<relative path>'` are watched. */
const watchNamedBySchema = z.strictObject({ "named-by": dataFile });

/** One rule's own paths, in place of the `{rule}` patterns of the kinds it names. */
const watchRuleSchema = z.strictObject({
  tests: unique(watchPattern(false)).min(1).optional(),
  code: unique(watchPattern(false)).min(1).optional(),
});

/**
 * What the nightly upstream report compares between the pinned commit and upstream's latest release (docs/pack-format.md,
 * "watch"): the tests the generator reads, the ported rules' code, the code they share, and where upstream adds tests
 * for a ported rule that the generator does not read. `{rule}` expands over the ported rules: conformance.json's
 * `upstream.rule` for this tool without `rule-prefix`, plus `extra-rules`.
 */
const watchSchema = z.strictObject({
  "rule-prefix": nonEmpty.optional(),
  "extra-rules": unique(nonEmpty).min(1).optional(),
  tests: unique(z.union([watchPattern(true), watchFileSchema, watchNamedBySchema])),
  code: unique(z.union([watchPattern(true), z.strictObject({ index: z.enum(WATCH_INDEXES) })])),
  shared: unique(watchPattern(false)),
  "new-tests": unique(watchPattern(true)).min(1).optional(),
  rules: z.record(nonEmpty, watchRuleSchema).optional(),
});
export type GatesWatch = z.infer<typeof watchSchema>;

export const gatesManifestSchema = z.strictObject({
  /** The upstream's linter family (ADAPTERS), which says where its releases are published. */
  adapter: z.enum(ADAPTERS),
  rerail: z.strictObject({
    /** The pack's id: the directory under packs/ this manifest sits in. */
    pack: z.string().regex(/^[a-z0-9-]+$/u),
    /**
     * The upstream project's directory under packs/<pack>/licenses/, which keeps its UPSTREAM.md and every licence file
     * at the root of upstream's repository under its own name.
     */
    project: z.string().regex(/^[A-Za-z0-9._-]+$/u),
    /**
     * The file types rerail's ports of this tool cover, when the pack deliberately covers fewer than upstream lints
     * (`upstream.lints`): `loads`'s in-scope check then asks for these alone. Each must be one of `upstream.lints`.
     * Absent, the ports must cover every type upstream lints.
     */
    in_scope: unique(z.string().regex(/^\.[a-z]+$/u)).min(1).optional(),
  }),
  upstream: z.strictObject({
    name: nonEmpty,
    repository: githubRepository,
    commit,
    /** The release the pins name; conformance.json's tools[].version must agree with it. */
    version: nonEmpty,
    license: nonEmpty,
    /** The upstream configurations that count as recommended, in order: a rule starts on when one turns it on (`upstream-status`). */
    recommended_configs: unique(nonEmpty).min(1),
    /** The directory under scripts/pack-data/runtimes/ that pins the runtime upstream is run with. */
    runtime: runtimeDirectory,
    /**
     * The file types upstream lints with its approved configuration at the pins. Every port of this tool must apply to
     * each of them (`loads`'s in-scope check), or to each of `rerail.in_scope` when the pack narrows it.
     */
    lints: unique(fileType).min(1),
  }),
  /** Recorded evidence that the upstream is public and popular: tool-pack registration data, not a per-rule label. */
  popularity: z.strictObject({
    observed_on: z.iso.date(),
    evidence: z.array(nonEmpty).min(1),
  }),
  /**
   * Every rule of the pinned upstream the pack ports or may port, by upstream name, with what upstream says of it. A
   * name not listed is no rule of the pinned upstream.
   */
  rules: z.record(nonEmpty, upstreamRuleFactsSchema),
  watch: watchSchema.optional(),
  eslint: eslintSchema.optional(),
}).superRefine((manifest, context) => {
  if ((manifest.adapter === "eslint") !== (manifest.eslint !== undefined)) {
    context.addIssue({ code: "custom", path: ["eslint"], message: "the eslint adapter needs the eslint block, and no other adapter takes one" });
  }
  for (const type of manifest.rerail.in_scope ?? []) {
    if (!manifest.upstream.lints.includes(type)) context.addIssue({ code: "custom", path: ["rerail", "in_scope"], message: `names ${type}, which upstream does not lint (upstream.lints: ${manifest.upstream.lints.join(", ")})` });
  }
});

export type GatesManifest = z.infer<typeof gatesManifestSchema>;

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

/** The tool-pack directory a gates.yaml sits in: `packs/<pack>/upstream/<tool>`, under `root`. */
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

/** Every gated tool-pack under `root`: a `packs/<pack>/upstream/<tool>/gates.yaml`, in path order. */
export function gatedToolPacks(root: string): { pack: string; tool: string }[] {
  const found: { pack: string; tool: string }[] = [];
  const packs = join(root, "packs");
  for (const pack of existsSync(packs) ? readdirSync(packs).sort() : []) {
    const upstream = join(packs, pack, "upstream");
    if (!existsSync(upstream)) continue;
    for (const tool of readdirSync(upstream).sort()) {
      if (existsSync(join(upstream, tool, GATES_MANIFEST))) found.push({ pack, tool });
    }
  }
  return found;
}

/**
 * The file types a port of the tool must apply to (`loads`'s in-scope check): `upstream.lints`, narrowed to
 * `rerail.in_scope` when the pack deliberately covers fewer, and to the port's own `lints` (conformance.json) when it
 * names them.
 */
export function lintedTypes(manifest: GatesManifest, port?: readonly string[]): string[] {
  const inScope = manifest.rerail.in_scope;
  return manifest.upstream.lints.filter((type) => (inScope === undefined || inScope.includes(type)) && (port === undefined || port.includes(type)));
}

/** One example path per file type of `lintedTypes`. */
export function lintedPaths(manifest: GatesManifest, port?: readonly string[]): string[] {
  return lintedTypes(manifest, port).map((type) => `src/example${type}`);
}
