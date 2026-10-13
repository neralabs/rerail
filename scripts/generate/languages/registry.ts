import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";

/**
 * The language registry (`languages/*.json`, format `rerail.language/v1`; see docs/language-packs.md): its schema, its
 * cross-entry rules, and the host module generated from it. The engine reads the same files in `native/build.rs`
 * (`native/src/build_languages.rs`), which applies the same rules, and a native test checks the generated module
 * against the engine's tables.
 */

const ROOT = resolve(fileURLToPath(import.meta.url), "..", "..", "..", "..");
export const LANGUAGES_DIRECTORY = resolve(ROOT, "languages");
export const LANGUAGE_REGISTRY_MODULE_PATH = resolve(ROOT, "src", "engine", "code-rule", "code-rule-language-registry.generated.ts");

export const LANGUAGE_REGISTRY_FORMAT = "rerail.language/v1";
/** The parser implementations a dialect may name. */
export const LANGUAGE_PARSERS = ["tree-sitter"] as const;

const name = z.string().regex(/^[a-z][a-z0-9-]*$/u, "Must be lowercase letters, digits and dashes, starting with a letter.");
const support = z.enum(["supported", "refused", "not-supported-yet"]);
const dialectSchema = z
  .object({
    label: name,
    variant: z.string().regex(/^[A-Z][A-Za-z0-9]*$/u, "Must be the engine's GrammarDialect variant (a Rust type name).").optional(),
    suffixes: z.array(z.string().regex(/^\.[^/A-Z]+$/u, "Must be a lowercase `.suffix`.")).min(1),
    parser: z.enum(LANGUAGE_PARSERS).optional(),
    grammar: z.string().regex(/^(?:vendored:[a-z][a-z0-9-]*|ast-grep:[A-Z][A-Za-z0-9]*)$/u, "Must be vendored:<name> or ast-grep:<Language>.").optional(),
    tapeTag: z.number().int().min(1).max(255).optional(),
  })
  .strict();

export const languageEntrySchema = z
  .object({
    format: z.literal(LANGUAGE_REGISTRY_FORMAT),
    id: name,
    status: z.enum(["supported", "planned"]),
    family: name,
    dialects: z.array(dialectSchema).min(1),
    runsOn: z.array(name).min(1).optional(),
    capabilities: z.record(z.string(), support),
    suppressions: z.array(name),
    embeds: z.array(z.object({ language: name, via: name }).strict()).optional(),
  })
  .strict()
  .superRefine((entry, context) => {
    const issue = (path: (string | number)[], message: string) => context.addIssue({ code: "custom", path, message });
    entry.dialects.forEach((dialect, index) => {
      if (entry.status !== "supported") return;
      for (const key of ["variant", "parser", "grammar", "tapeTag"] as const) {
        if (dialect[key] === undefined) issue(["dialects", index, key], `A supported dialect needs a ${key}; a supported language needs a grammar for every dialect.`);
      }
    });
    if (entry.status === "planned") {
      if (entry.runsOn !== undefined) issue(["runsOn"], "A planned language runs on no dialect yet.");
      if (Object.values(entry.capabilities).includes("supported")) issue(["capabilities"], "A planned language supports no capability yet.");
    }
    if (new Set(entry.suppressions).size !== entry.suppressions.length) issue(["suppressions"], "Suppressions repeat.");
    for (const embed of entry.embeds ?? []) if (embed.language === entry.id) issue(["embeds"], "A language does not embed itself.");
  });

export type LanguageEntry = z.infer<typeof languageEntrySchema>;

/** Cross-entry rules: unique ids, labels, suffixes and tape tags; one capability vocabulary; runsOn and embeds resolve; a family's languages share its suppressions. */
export function languageRegistryProblems(entries: readonly { file: string; entry: LanguageEntry }[]): string[] {
  const problems: string[] = [];
  const owners = new Map<string, string>();
  const claim = (kind: string, key: string, file: string) => {
    const other = owners.get(`${kind}:${key}`);
    if (other === undefined) owners.set(`${kind}:${key}`, file);
    else problems.push(`${file}: ${kind} ${key} is also ${other}'s`);
  };
  const ids = new Set(entries.map(({ entry }) => entry.id));
  const capabilities = JSON.stringify(Object.keys(entries[0]?.entry.capabilities ?? {}).toSorted());
  const dialects = new Map(entries.flatMap(({ entry }) => entry.dialects.map(dialect => [dialect.label, entry] as const)));
  for (const { file, entry } of entries) {
    if (`${entry.id}.json` !== basename(file)) problems.push(`${file}: id ${entry.id} must equal the file name`);
    claim("id", entry.id, file);
    if (JSON.stringify(Object.keys(entry.capabilities).toSorted()) !== capabilities) problems.push(`${file}: capabilities must name the same capabilities as ${entries[0]!.file}`);
    for (const dialect of entry.dialects) {
      claim("dialect label", dialect.label, file);
      for (const suffix of dialect.suffixes) claim("suffix", suffix, file);
      if (dialect.tapeTag !== undefined) claim("tape tag", String(dialect.tapeTag), file);
    }
    for (const embed of entry.embeds ?? []) if (!ids.has(embed.language)) problems.push(`${file}: embeds unknown language ${embed.language}`);
    if (entry.status === "supported") {
      for (const embed of entry.embeds ?? []) {
        const embedded = entries.find(({ entry: candidate }) => candidate.id === embed.language)?.entry;
        if (embedded && embedded.status !== "supported") {
          problems.push(
            `${file}: embeds planned language ${embed.language}; a supported language embeds only supported ones`,
          );
        }
      }
    }
    if (entry.status !== "supported") continue;
    const own = entry.dialects.map(dialect => dialect.label);
    const runsOn = entry.runsOn ?? own;
    if (runsOn[0] !== own[0]) problems.push(`${file}: runsOn starts with the language's first dialect, ${own[0]}`);
    if (new Set(runsOn).size !== runsOn.length) problems.push(`${file}: runsOn repeats a dialect`);
    for (const label of own) if (!runsOn.includes(label)) problems.push(`${file}: runsOn omits its own dialect ${label}`);
    for (const label of runsOn) {
      const owner = dialects.get(label);
      if (owner?.status !== "supported" || owner.family !== entry.family) problems.push(`${file}: runsOn dialect ${label} is not a supported dialect of family ${entry.family}`);
    }
    for (const { file: other, entry: sibling } of entries) {
      if (sibling.status === "supported" && sibling.family === entry.family && JSON.stringify(sibling.suppressions) !== JSON.stringify(entry.suppressions)) {
        problems.push(`${file} and ${other} share family ${entry.family} but not its suppressions`);
      }
    }
  }
  return problems;
}

/** Supported languages in the order they were added (their first dialect's tape tag), then planned ones by id. */
function compareEntries(left: LanguageEntry, right: LanguageEntry): number {
  const rank = (entry: LanguageEntry) => (entry.status === "supported" ? entry.dialects[0]!.tapeTag! : 256);
  return rank(left) - rank(right) || (left.id < right.id ? -1 : left.id > right.id ? 1 : 0);
}

/** Every registry file in `directory`, parsed, validated, and in registry order; throws naming each problem. */
export function readLanguageRegistry(directory = LANGUAGES_DIRECTORY): LanguageEntry[] {
  const problems: string[] = [];
  const entries: { file: string; entry: LanguageEntry }[] = [];
  for (const file of readdirSync(directory).filter(file => file.endsWith(".json")).toSorted()) {
    const parsed = languageEntrySchema.safeParse(JSON.parse(readFileSync(resolve(directory, file), "utf8")));
    if (parsed.success) entries.push({ file: `languages/${file}`, entry: parsed.data });
    else problems.push(...parsed.error.issues.map(issue => `languages/${file}: ${issue.path.join("/") || "(root)"}: ${issue.message}`));
  }
  if (entries.length === 0 && problems.length === 0) problems.push("languages/ holds no language");
  if (problems.length === 0) problems.push(...languageRegistryProblems(entries));
  if (problems.length > 0) throw new Error(`Invalid language registry:\n${problems.join("\n")}`);
  return entries.map(({ entry }) => entry).toSorted(compareEntries);
}

const literalList = (values: readonly string[]) => `[${values.map(value => JSON.stringify(value)).join(", ")}]`;

/** `src/engine/code-rule/code-rule-language-registry.generated.ts`: the registry, and the named lists the host's types and schemas read. */
export function renderLanguageRegistryModule(entries: readonly LanguageEntry[] = readLanguageRegistry()): string {
  const supported = entries.filter(entry => entry.status === "supported");
  const unique = (values: readonly string[]) => [...new Set(values)];
  return [
    "// Generated by scripts/generate/languages/registry.ts; do not edit. Source: languages/*.json (`npm run generate:languages`).",
    "",
    "/** The language registry (rerail.language/v1): supported languages in the order they were added, then planned ones. */",
    `export const LANGUAGE_REGISTRY = /* registry */ ${JSON.stringify(entries, null, 2)} /* end registry */ as const;`,
    "",
    "/** Every language a rule may name: supported, then planned. */",
    `export const CODE_RULE_LANGUAGE_IDS = ${literalList(entries.map(entry => entry.id))} as const;`,
    "/** The languages the engine reads. */",
    `export const CODE_RULE_AST_LANGUAGES = ${literalList(supported.map(entry => entry.id))} as const;`,
    "/** The languages a rule may name that the engine does not read yet. */",
    `export const CODE_RULE_PLANNED_LANGUAGES = ${literalList(entries.filter(entry => entry.status === "planned").map(entry => entry.id))} as const;`,
    "/** The semantic families of the supported languages. */",
    `export const CODE_RULE_LANGUAGE_FAMILIES = ${literalList(unique(supported.map(entry => entry.family)))} as const;`,
    "/** The capabilities every entry declares support for. */",
    `export const CODE_RULE_SEMANTIC_CAPABILITIES = ${literalList(Object.keys(entries[0]?.capabilities ?? {}))} as const;`,
    "/** The suppression comments the supported languages honour. */",
    `export const CODE_RULE_SUPPRESSIONS = ${literalList(unique(supported.flatMap(entry => entry.suppressions)))} as const;`,
    "",
  ].join("\n");
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  writeFileSync(LANGUAGE_REGISTRY_MODULE_PATH, renderLanguageRegistryModule());
  process.stdout.write(`Wrote ${LANGUAGE_REGISTRY_MODULE_PATH}\n`);
}
