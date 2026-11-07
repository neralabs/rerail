/**
 * The blade pack's derivation recipes: each tool's generate.py (packs/blade/upstream/<tool>/, beside common.py) run on
 * upstream's repository at the gates manifest's commit and the Composer project the port readiness recorder installs
 * from the tool's runtime (runtimes/<gates.yaml upstream.runtime>/: the same packages at the same exact versions), in
 * `fixtures` mode and then `corpus` mode over every corpus the pack pins. The generators run `php` from PATH, which must
 * be the pinned PHP build (runtimes/php/build.sh) the recorder refuses any other than.
 */
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { composerRuntime, PHP_INI, pinnedPhp, RUNTIMES_DIRECTORY } from "../../../conformance/recorders/port-readiness/runtimes.js";
import { derivationEnvironment, type DeriveContext, type DeriveRecipe } from "../context.js";

/** The generators use Python's standard library alone, on the interpreter the recorder's Python runtime pins. */
const PYTHON = readFileSync(join(RUNTIMES_DIRECTORY, "python", ".python-version"), "utf8").trim();

/** `name=<checkout>` for every corpus the pack pins, in conformance.json's order (generate.py's `corpus` arguments). */
async function corpusArguments(context: DeriveContext): Promise<string[]> {
  const directory = await context.corpora();
  const manifest = JSON.parse(readFileSync(join(context.root, "packs", context.pack, "conformance.json"), "utf8")) as { corpora: { name: string }[] };
  return manifest.corpora.map(({ name }) => `${name}=${join(directory, name)}`);
}

/**
 * tlint and stillat/blade-parser: `generate.py fixtures <checkout> <project>`, then `generate.py corpus <project>
 * <name>=<corpus>...`, the project the recorder's Composer runtime for the tool.
 */
function projectRecipe(tool: string): DeriveRecipe {
  return {
    pack: "blade",
    tool,
    async derive(context) {
      const { upstream } = readGatesManifest(context.root, context.pack, tool);
      const checkout = await context.checkout(upstream.repository, upstream.commit);
      const project = await composerRuntime(upstream.runtime, context.recorderCache, context.runner);
      const interpreter = await context.python(PYTHON);
      const generator = join(context.toolDirectory, "generate.py");
      await context.exec([interpreter, generator, "fixtures", checkout, project.directory]);
      await context.exec([interpreter, generator, "corpus", project.directory, ...(await corpusArguments(context))]);
    },
  };
}

/**
 * Bladestan: generate.py runs Bladestan's own rule test from a checkout of it with its development dependencies
 * installed and PHPStan pinned (its docstring: `composer update && composer require --dev phpstan/phpstan:2.2.16 -W`).
 * The checkout's files are copied into a cache directory and its composer.json given the bladestan runtime's exact
 * versions (runtimes/<runtime>/composer.json, the versions the docstring names) for the packages it requires, each in
 * the section upstream lists it in (require-dev for one it does not), then resolved as the recorder resolves a runtime.
 */
const bladestan: DeriveRecipe = {
  pack: "blade",
  tool: "bladestan",
  async derive(context) {
    const { upstream } = readGatesManifest(context.root, context.pack, "bladestan");
    const generator = join(context.toolDirectory, "generate.py");
    const origin = /^ORIGIN = "([^"]+)"$/mu.exec(readFileSync(generator, "utf8"))?.[1];
    if (origin !== `${upstream.repository}@${upstream.commit}`) throw new Error(`${generator} records ${origin ?? "no ORIGIN"}; gates.yaml pins ${upstream.repository}@${upstream.commit}`);
    const checkout = await context.checkout(upstream.repository, upstream.commit);

    const runtime = JSON.parse(readFileSync(join(RUNTIMES_DIRECTORY, upstream.runtime, "composer.json"), "utf8")) as { require: Record<string, string> };
    const manifest = JSON.parse(readFileSync(join(checkout, "composer.json"), "utf8")) as { name: string; require: Record<string, string>; "require-dev": Record<string, string> };
    for (const [name, version] of Object.entries(runtime.require)) {
      if (name === manifest.name) continue;
      if (name in manifest.require) manifest.require[name] = version;
      else manifest["require-dev"][name] = version;
    }
    const text = `${JSON.stringify(manifest, null, 4)}\n`;
    const { php } = await pinnedPhp(upstream.runtime, context.runner);
    const pin = createHash("sha256").update(`${upstream.commit}\0${text}\0${readFileSync(join(RUNTIMES_DIRECTORY, upstream.runtime, ".php-version"), "utf8")}`).digest("hex");
    const project = join(context.cacheRoot, "tools", `bladestan-${pin.slice(0, 16)}`);
    const marker = join(project, ".rerail-derive-pin");
    if (!existsSync(marker) || readFileSync(marker, "utf8") !== pin) {
      context.log(`installing Bladestan ${upstream.version} with its development dependencies in ${project}`);
      await rm(project, { recursive: true, force: true });
      await mkdir(project, { recursive: true });
      // Every tracked file: `git archive` leaves out tests/, which upstream's .gitattributes marks export-ignore.
      await context.exec(["sh", "-c", 'git -C "$1" ls-files -z | tar -C "$1" --null -T - -cf - | tar -xf - -C "$2"', "sh", checkout, project]);
      await writeFile(join(project, "composer.json"), text);
      const composer = (await context.capture(["sh", "-c", "command -v composer"])).trim();
      await context.exec([php, ...PHP_INI, composer, "update", "--no-interaction", "--no-progress", "--no-plugins", "--no-scripts", "--prefer-dist", "--no-blocking", "--audit-format=plain"], { cwd: project, env: { ...derivationEnvironment(), COMPOSER_NO_INTERACTION: "1" } });
      await writeFile(marker, pin);
    }

    const interpreter = await context.python(PYTHON);
    await context.exec([interpreter, generator, "fixtures", project]);
    await context.exec([interpreter, generator, "corpus", project, ...(await corpusArguments(context))]);
  },
};

export const recipes: DeriveRecipe[] = [projectRecipe("tlint"), bladestan, projectRecipe("blade-parser")];
