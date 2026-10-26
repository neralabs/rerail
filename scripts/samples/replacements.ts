/**
 * The permissive replacements for the three rules whose only real-code positive was on a copyleft golden (D1,
 * copyleft-goldens.md §4). Each file is pinned by repository, commit, path and sha256, and labelled only for the rule
 * it replaces evidence for, with the lines the pinned linter reported when run on it by hand as copyleft-goldens.md
 * §4.1 ran it (and as the tool-pack's recorder configures it):
 *
 * - SQLFluff 4.4.0, `lint_string` with dialect postgres, templater raw, the rule alone;
 * - eslint-plugin-react-hooks 7.1.1 on ESLint 9 with @typescript-eslint/parser 8.70.1, JSX on, inline disables honoured;
 * - Bandit 1.9.4, `-t B304`, default profile, `# nosec` honoured (Bandit's column is a 0-based byte offset, + 1 here).
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { packRuleIds, type PackConformance } from "./corpora.js";
import type { SampleLabel } from "./format.js";
import { sha256, type Sample, type ToolPack } from "./samples.js";

export interface Replacement {
  toolPack: ToolPack;
  /** The upstream rule labelled. */
  rule: string;
  repo: string;
  commit: string;
  path: string;
  sha256: string;
  version: string;
  labels: readonly SampleLabel[];
}

const SHADCN = { repo: "shadcn-ui/ui", commit: "0e3abd65a97707f4a9cc3ed07bf5006e1cb67b13" };
const HOOKS = { toolPack: { pack: "react", tool: "eslint-plugin-react-hooks" }, rule: "react-hooks/exhaustive-deps", version: "7.1.1", ...SHADCN };
const at = (lines: readonly [number, number][]): SampleLabel[] => lines.map(([line, column]) => ({ line, column }));

export const REPLACEMENTS: readonly Replacement[] = [
  {
    toolPack: { pack: "sql", tool: "sqlfluff" },
    rule: "ST01",
    repo: "postgres/postgres",
    commit: "724edf9bde9d356724ad384a2e196edc3c9f80f7",
    path: "src/test/modules/test_predtest/sql/test_predtest.sql",
    sha256: "5ee08b2684b6705fff451cbf2dd86c862dc9eb147983f36e43f6b348c433f171",
    version: "4.4.0",
    labels: at([[11, 3], [12, 3], [13, 3], [14, 3]]),
  },
  {
    ...HOOKS,
    path: "packages/react/src/message-scroller/use-message-scroller-commands.ts",
    sha256: "6ee8c7afee27689114a37e41ca134f365d3aeaeb5958e751bb507b7bf3a1f315",
    labels: at([[69, 5], [89, 6], [128, 5], [145, 5], [169, 5], [230, 5], [251, 6], [290, 5], [316, 6]]),
  },
  {
    ...HOOKS,
    path: "packages/react/src/message-scroller/use-message-scroller-controller.ts",
    sha256: "af5d68bb4b0d87336cab78947cc0f0638e72f5acf4ba4d6b3a7f947d76a883f8",
    labels: at([
      [138, 5], [176, 5], [201, 6], [212, 6], [244, 6], [286, 6], [309, 6], [323, 6], [381, 6], [493, 6], [535, 6],
      [591, 6], [603, 6], [629, 5], [643, 6], [657, 5], [665, 5], [695, 5], [723, 6], [753, 6], [766, 6],
    ]),
  },
  {
    ...HOOKS,
    path: "packages/react/src/questionnaire/use-questionnaire-item.ts",
    sha256: "8c0070e384e10ac8730b046ccfb2a5ee6655524680e6a58eb807213936b92e3f",
    labels: at([[72, 5]]),
  },
  {
    ...HOOKS,
    path: "packages/react/src/questionnaire/use-questionnaire-root.ts",
    sha256: "17e520e71d23ddb25d227ba705a8ae6fd109bb1397077bf5155bfca06ccb33ae",
    labels: at([[132, 5], [485, 5]]),
  },
  {
    toolPack: { pack: "python", tool: "bandit" },
    rule: "B304",
    repo: "Legrandin/pycryptodome",
    commit: "5c2dfecbeba8d596fb4a369f6e9b18971269498a",
    path: "lib/Crypto/IO/PEM.py",
    sha256: "dfed992b76ddfa5868a83cedf54b5b4a4a8b6d5be79544275aeb2fbef8fbeaad",
    version: "1.9.4",
    labels: at([[149, 22]]),
  },
];

/** A pinned file's bytes: read once from GitHub into the builder's cache, then from there; checked against its sha256. */
export function pinnedFile(cacheRoot: string, file: Pick<Replacement, "repo" | "commit" | "path" | "sha256">): Buffer {
  const cached = join(cacheRoot, "sources", file.repo.toLowerCase(), file.commit, file.path);
  if (!existsSync(cached)) {
    const bytes = execFileSync("gh", ["api", "-H", "Accept: application/vnd.github.raw", `repos/${file.repo}/contents/${file.path}?ref=${file.commit}`], { maxBuffer: 64 * 1024 * 1024 });
    mkdirSync(dirname(cached), { recursive: true });
    writeFileSync(cached, bytes);
  }
  const bytes = readFileSync(cached);
  if (sha256(bytes) !== file.sha256) throw new Error(`${file.repo} ${file.path} at ${file.commit} is ${sha256(bytes)}, not the pinned ${file.sha256}`);
  return bytes;
}

export function replacementSamples(options: {
  cacheRoot: string;
  conformances: ReadonlyMap<string, PackConformance>;
  licence: (repo: string, commit: string) => string;
}): Sample[] {
  return REPLACEMENTS.map((replacement) => {
    const ids = packRuleIds(options.conformances.get(replacement.toolPack.pack)!, replacement.toolPack.tool, replacement.rule);
    if (ids.length === 0) throw new Error(`${replacement.path}: no rule of ${replacement.toolPack.pack} ports ${replacement.rule}`);
    return {
      toolPack: replacement.toolPack,
      short: replacement.repo.split("/")[1]!.toLowerCase(),
      source: { repo: replacement.repo, commit: replacement.commit, path: replacement.path, license: options.licence(replacement.repo, replacement.commit), sha256: replacement.sha256 },
      bytes: pinnedFile(options.cacheRoot, replacement),
      labelledBy: { tool: replacement.toolPack.tool, version: replacement.version, how: "hand-labelled" as const },
      expected: new Map(ids.map((id) => [id, replacement.labels])),
      context: [],
      pinned: true,
      origins: ["replacement" as const],
    };
  });
}
