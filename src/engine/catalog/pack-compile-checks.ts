/**
 * The engine's verdict on each installed pack's rules, kept in the Rerail home so that a process need not compile every
 * rule again to learn it. The catalog checks a pack by compiling each of its code rules (`readRuleCatalog` with
 * `compile`): for the Node, TypeScript, JavaScript, Python and complexity packs that is 767 rules, 14 s of CPU and
 * 1.7 GB on a Windows laptop, paid by every command that reads the catalog and at every daemon start. A verdict holds for
 * one pack's content and core version (the key the catalog gives) and one engine build: the native addon and the
 * capabilities rules compile against. Any change to either asks the engine again. Beside the verdicts it keeps the
 * engine's reading of each `.rerail` pack rule (`kept-rerail-descriptions.ts`).
 */
import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { z } from "zod";
import { bundledCodeRuleDslDirectory } from "../code-rule/code-rule-compile.js";
import { replaceFileSync } from "../file-replace.js";
import { resolveNativeBindingPath } from "../native/native.js";
import { keptDescriptionSchema, keptRerailDescriber } from "./rerail-rules/kept-rerail-descriptions.js";
import { describeRerailRules, type RerailDescriber, type RerailDescription } from "./rerail-rules/rerail-rule.js";

/** In the Rerail home: the verdicts of the last catalog read, per engine build. */
export const PACK_COMPILE_CHECKS_FILE = "pack-compile-checks.json";

/**
 * Engine builds whose verdicts are kept, the latest first: a Git checkout's build and the managed installation can
 * share one home, and neither should drop the other's.
 */
const KEPT_BUILDS = 4;

/**
 * Per engine build, each pack's verdict: null when every rule compiled, else why the pack is left out; under `notRun`,
 * the ids of its rules this Rerail does not run (`readsLeftOutToken`); and the engine's reading of each `.rerail` rule
 * file, by its path. `notRun` is a field of its own so that a Rerail that predates it still reads the file; a verdict
 * without it was kept by such a Rerail, and is asked again.
 */
const KeptChecksSchema = z.array(z.object({
  build: z.string(),
  verdicts: z.record(z.string(), z.string().nullable()),
  notRun: z.record(z.string(), z.array(z.string())).optional(),
  descriptions: z.record(z.string(), keptDescriptionSchema).optional(),
}));
type KeptChecks = z.infer<typeof KeptChecksSchema>;

/** The engine's verdict on one pack: why it is left out (null when it loads), and the ids of its rules it does not run. */
export interface PackVerdict {
  readonly problem: string | null;
  readonly notRun: readonly string[];
}

export interface PackCompileChecks {
  /** The verdict kept on the pack `packKey` names, or undefined when the engine was never asked under this build. */
  verdict(packKey: string): PackVerdict | undefined;
  record(packKey: string, verdict: PackVerdict): void;
  /** Reads `.rerail` rules as `describeRerailRules` does, from the readings kept under this build where their files are unchanged. */
  readonly describe: RerailDescriber;
  /** Writes back the verdicts and readings this read used or made, dropping this build's others, when they differ from the file's. */
  save(): void;
}

/** The engine build verdicts hold for: the native addon by path and stamp, and the capabilities rules compile against. */
function engineBuild(): string | undefined {
  try {
    const addon = resolveNativeBindingPath();
    const { mtimeMs, size } = statSync(addon);
    const capabilities = readFileSync(resolve(bundledCodeRuleDslDirectory(), "capabilities.json"));
    return createHash("sha256").update(`${addon}\0${mtimeMs}:${size}\0`).update(capabilities).digest("hex");
  } catch {
    // Without the engine nothing compiles, and the catalog says so; there is nothing to keep.
    return undefined;
  }
}

function readKept(path: string): KeptChecks {
  try {
    return KeptChecksSchema.parse(JSON.parse(readFileSync(path, "utf8")));
  } catch {
    // Missing, unreadable or not ours: the engine is asked again, and `save` replaces it.
    return [];
  }
}

function writeKept(path: string, checks: KeptChecks): void {
  try {
    replaceFileSync(path, `${JSON.stringify(checks, null, 2)}\n`, { mode: 0o600 });
  } catch {
    // A verdict not kept costs the next process a compile, never a wrong answer; replaceFileSync removed its new file.
  }
}

/** The verdicts kept in `home` for the running engine build, for one catalog read. */
export function packCompileChecks(home: string): PackCompileChecks {
  const path = join(home, PACK_COMPILE_CHECKS_FILE);
  const build = engineBuild();
  const all = readKept(path);
  const mine = all.find(checks => checks.build === build);
  const kept = new Map<string, PackVerdict>();
  for (const [packKey, problem] of Object.entries(mine?.verdicts ?? {})) {
    const notRun = mine?.notRun?.[packKey];
    if (notRun !== undefined) kept.set(packKey, { problem, notRun });
  }
  const keptDescriptions = new Map(Object.entries(mine?.descriptions ?? {}));
  const used = new Map<string, PackVerdict>();
  const usedDescriptions = new Map<string, RerailDescription>();
  let recorded = false;
  const keep = (rule: string, description: RerailDescription): void => {
    usedDescriptions.set(rule, description);
    recorded ||= keptDescriptions.get(rule) !== description;
  };
  return {
    verdict(packKey) {
      const verdict = build === undefined ? undefined : kept.get(packKey);
      if (verdict !== undefined) used.set(packKey, verdict);
      return verdict;
    },
    record(packKey, verdict) {
      if (build === undefined) return;
      used.set(packKey, verdict);
      recorded = true;
    },
    describe: build === undefined ? describeRerailRules : keptRerailDescriber(keptDescriptions, keep),
    save() {
      if (build === undefined || (!recorded && used.size === kept.size && usedDescriptions.size === keptDescriptions.size)) return;
      const others = all.filter(checks => checks.build !== build).slice(0, KEPT_BUILDS - 1);
      const verdicts = Object.fromEntries([...used].map(([packKey, verdict]) => [packKey, verdict.problem]));
      const notRun = Object.fromEntries([...used].map(([packKey, verdict]) => [packKey, [...verdict.notRun]]));
      writeKept(path, [{ build, verdicts, notRun, descriptions: Object.fromEntries(usedDescriptions) }, ...others]);
    },
  };
}
