import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { chmod, lstat, mkdir, open, realpath, rename, rm } from "node:fs/promises";
import { join } from "node:path";
import { experimentSchema, scenarioSchema, type PlaygroundExperiment, type PlaygroundScenario } from "./protocol.js";

const MAX_EXPERIMENT_BYTES = 72 * 1024 * 1024;
const MAX_FIXTURE_BYTES = 2 * 1024 * 1024;
const RESERVED_PATHS: Record<string, true> = { ".rerail": true, ".playground-home": true, ".git": true };
/** Shared by generated, edited, persisted, and executed scenarios; fixtures are never host paths. */
export function validateScenarioFixtures(scenario: PlaygroundScenario): void {
  scenarioSchema.parse(scenario);
  const paths = new Set<string>();
  let bytes = 0;
  for (const fixture of scenario.fixtures) {
    const parts = fixture.path.split("/");
    if (
      fixture.path.includes("\\") ||
      fixture.path.includes(":") ||
      /\p{Cc}/u.test(fixture.path) ||
      parts.some((part) => !part || part === "." || part === "..") ||
      Object.hasOwn(RESERVED_PATHS, parts[0]!.normalize("NFC").toLowerCase())
    ) {
      throw new Error(`Fixture must use a non-reserved sandbox-relative path: ${fixture.path}`);
    }
    const key = fixture.path.normalize("NFC").toLowerCase();
    if (paths.has(key)) throw new Error(`Duplicate fixture path: ${fixture.path}`);
    paths.add(key);
    bytes += Buffer.byteLength(fixture.content, "utf8");
    if (bytes > MAX_FIXTURE_BYTES) throw new Error("Scenario fixtures exceed the 2 MiB aggregate limit");
  }
  for (const path of paths) {
    const parts = path.split("/");
    for (let index = 1; index < parts.length; index++) {
      if (paths.has(parts.slice(0, index).join("/")))
        throw new Error(`Fixture file conflicts with a parent directory: ${path}`);
    }
  }
}

function validateExperiment(value: unknown, ruleId?: string): PlaygroundExperiment {
  const experiment = experimentSchema.parse(value);
  if (
    !experiment.ruleId ||
    experiment.ruleId !== experiment.selection.ruleId ||
    (ruleId !== undefined && experiment.ruleId !== ruleId)
  )
    throw new Error("Experiment rule identity is inconsistent");
  const ids = new Set<string>();
  for (const scenario of experiment.scenarios) {
    validateScenarioFixtures(scenario);
    if (ids.has(scenario.id)) throw new Error(`Duplicate scenario id: ${scenario.id}`);
    ids.add(scenario.id);
  }
  if (experiment.lastRun) {
    const run = experiment.lastRun;
    validateScenarioFixtures(run.scenario);
    if (
      run.rule.id !== experiment.ruleId ||
      run.selection.ruleId !== experiment.ruleId ||
      run.selection.scenarioId !== run.scenario.id
    )
      throw new Error("Persisted run snapshot identity is inconsistent");
  }
  return experiment;
}

function experimentFilename(ruleId: string): string {
  if (!ruleId || ruleId.length > 128) throw new Error("Invalid experiment rule id");
  return `${createHash("sha256").update(ruleId).digest("hex")}.json`;
}

/** Checkout-local state, deliberately unrelated to the live rules/session database. */
export function createExperimentStore(root: string): {
  load(ruleId: string): Promise<PlaygroundExperiment | undefined>;
  save(experiment: PlaygroundExperiment): Promise<void>;
} {
  async function directory(): Promise<string> {
    let path = await realpath(root);
    for (const segment of [".rerail", "playground"]) {
      path = join(path, segment);
      await mkdir(path, { mode: 0o700 }).catch((error: NodeJS.ErrnoException) => {
        if (error.code !== "EEXIST") throw error;
      });
      const entry = await lstat(path);
      if (!entry.isDirectory() || entry.isSymbolicLink())
        throw new Error(`Experiment storage must be a real directory: ${path}`);
    }
    await chmod(path, 0o700);
    return path;
  }
  return {
    async load(ruleId) {
      const path = join(await directory(), experimentFilename(ruleId));
      let file;
      try {
        file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
        throw error;
      }
      try {
        const stat = await file.stat();
        if (!stat.isFile() || stat.size > MAX_EXPERIMENT_BYTES)
          throw new Error("Experiment file is invalid or exceeds its size limit");
        const value: unknown = JSON.parse(await file.readFile("utf8"));
        // An experiment saved under an older protocol cannot be replayed; the next save replaces it.
        if (!experimentSchema.safeParse(value).success) return undefined;
        return validateExperiment(value, ruleId);
      } finally {
        await file.close();
      }
    },
    async save(value) {
      const experiment = validateExperiment(value);
      const content = JSON.stringify(experiment);
      if (Buffer.byteLength(content, "utf8") > MAX_EXPERIMENT_BYTES)
        throw new Error("Experiment exceeds its storage size limit");
      const dir = await directory();
      const target = join(dir, experimentFilename(experiment.ruleId));
      const temporary = join(dir, `.${randomUUID()}.tmp`);
      try {
        const file = await open(temporary, "wx", 0o600);
        try {
          await file.writeFile(content, "utf8");
          await file.sync();
        } finally {
          await file.close();
        }
        await rename(temporary, target);
        const parent = await open(dir, constants.O_RDONLY);
        try {
          await parent.sync();
        } finally {
          await parent.close();
        }
      } finally {
        await rm(temporary, { force: true });
      }
    },
  };
}
