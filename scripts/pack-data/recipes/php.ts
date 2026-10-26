/**
 * The php pack's derivation recipes: PHP_CodeSniffer's, PHPStan's and Psalm's generate.py (with
 * packs/php/upstream/common.py), each run with the Composer project of the recorder's runtimes/<tool>-<version> (the
 * runtime gates.yaml names) on the PHP that runtime pins (runtimes/php's build, which composerRuntime refuses any other
 * than, put first on PATH for the generator), over upstream's checkout at the generator's ORIGIN.
 */
import { readFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { z } from "zod";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { composerRuntime, PHP_INI, pinnedPhp, RUNTIMES_DIRECTORY, uvRuntime } from "../../../conformance/recorders/port-readiness/runtimes.js";
import { derivationEnvironment, type DeriveContext, type DeriveRecipe } from "../context.js";

/** The generator's ORIGIN (`<repository>@<commit>`), read from it so a pin bump needs no recipe edit. */
function generatorOrigin(generator: string): { repository: string; commit: string } {
  const origin = /^ORIGIN = "([^"@]+)@([0-9a-f]{40})"$/mu.exec(readFileSync(generator, "utf8"));
  if (origin === null) throw new Error(`${generator}: no ORIGIN = "<repository>@<commit>"`);
  return { repository: origin[1]!, commit: origin[2]! };
}

const composerManifest = z.object({ require: z.record(z.string(), z.string()) });

interface PhpDerivation {
  /** Upstream's paths (sparse) the `fixtures` mode reads. */
  sparse: readonly string[];
  /** Recorder runtimes whose requirements the generator's Composer project needs beside the tool's own (the recorder keeps them apart). */
  alongside?: readonly string[];
}

/**
 * Runs the tool's generate.py in its `fixtures` mode. The generator's one Composer project is the runtime gates.yaml
 * names.
 */
async function derivePhp(context: DeriveContext, { sparse, alongside = [] }: PhpDerivation): Promise<void> {
  const { root, pack, tool, toolDirectory, recorderCache, runner } = context;
  const generator = join(toolDirectory, "generate.py");
  const origin = generatorOrigin(generator);
  const { upstream } = readGatesManifest(root, pack, tool);
  if (upstream.runtime === undefined) throw new Error(`packs/${pack}/upstream/${tool}/gates.yaml names no runtime`);
  const runtime = upstream.runtime;
  // The PHP runtimes/<runtime>/.php-version pins, with runtimes/php's extensions: any other throws.
  const { php } = await pinnedPhp(runtime, runner);
  const project = alongside.length === 0
    ? (await composerRuntime(runtime, recorderCache, runner)).directory
    : await context.composerTools(`${runtime}-with-${alongside.join("-")}`, {
      // The exact requirements of the recorder's runtimes/<name>/composer.json, together.
      require: Object.fromEntries([runtime, ...alongside].flatMap((name) => Object.entries(composerManifest.parse(JSON.parse(readFileSync(join(RUNTIMES_DIRECTORY, name, "composer.json"), "utf8"))).require))),
    });
  // The generators run `php` (their readers) and vendor/bin scripts (`#!/usr/bin/env php`) from PATH: the pinned
  // interpreter the runtime checked, with the settings every PHP process of the gates runs with (PHP_INI) as its
  // php.ini, so the build's own built-in defaults (short_open_tag on: PHP_CodeSniffer reports no Internal.NoCodeFound
  // for a file with no open tag) answer nothing.
  const ini = await context.scratch("php-ini");
  const settings = PHP_INI.flatMap((argument, index) => (PHP_INI[index - 1] === "-d" ? [`${argument}\n`] : []));
  await writeFile(join(ini, "php.ini"), settings.join(""));
  const env = { ...derivationEnvironment(), PATH: `${dirname(php)}:${process.env.PATH ?? ""}`, PHPRC: ini, PHP_INI_SCAN_DIR: "" };
  // The generators need only the standard library: the recorder's runtimes/python interpreter runs them.
  const python = (await uvRuntime("python", recorderCache, runner)).python;
  const checkout = await context.checkout(origin.repository, origin.commit, { sparse });
  await context.exec([python, generator, "fixtures", checkout, project], { env });
}

export const recipes: DeriveRecipe[] = [
  {
    pack: "php",
    tool: "phpcs",
    derive: (context) => derivePhp(context, { sparse: ["/src/Standards/"] }),
  },
  {
    pack: "php",
    tool: "phpstan",
    // read_tests.php reads the tests with nikic/php-parser (runtimes/php's), which the phpstan phar does not autoload.
    derive: (context) => derivePhp(context, { sparse: ["/tests/PHPStan/"], alongside: ["php"] }),
  },
  {
    pack: "php",
    tool: "psalm",
    derive: (context) => derivePhp(context, { sparse: ["/tests/"] }),
  },
];
