/**
 * The PHP and Blade ports are candidates (packs/<id>/candidates/rules/), so pack.json lists no rules and check 3 of the
 * pack validation contract no longer runs the example pairs some of them carried while they shipped. Each pack's README
 * keeps those pairs under `### \`<rule id>\``, a `Caught (\`<path>\`):` block and a `Passes (\`<path>\`):` block; this
 * holds every pair to check 3 against the candidate's document: the caught example produces one of the rule's
 * violation codes, the passes example none. Blade's pairs run its embedded PHP through the multi-region embedding.
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { z } from "zod";
import { evaluateExample } from "../../../conformance/pack-validation.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { repositoryPath } from "../../support/oracles.js";

const PACKS = ["php", "blade"] as const;
const capabilities = readFileSync(repositoryPath("native", "src", "code_rule_dsl", "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;

const documentSchema = z.object({ clauses: z.array(z.looseObject({ violation: z.object({ code: z.string() }).optional() })) });

interface Example {
  path: string;
  text: string;
}

/**
 * The example pairs in a pack README's `## Candidates` section (its `## Rule details` holds prose, under the same
 * headings). Every `### ` section there is one pair: a heading that is not exactly `` `<rule id>` ``, or a section
 * without both blocks, fails rather than dropping the pair.
 */
function readmePairs(pack: string): { id: string; caught: Example; passes: Example }[] {
  const readme = readFileSync(repositoryPath("packs", pack, "README.md"), "utf8");
  const candidates = /^## Candidates\n([\s\S]*?)(?=^## |(?![\s\S]))/mu.exec(readme)?.[1];
  if (candidates === undefined) throw new Error(`packs/${pack}/README.md: has no ## Candidates section`);
  return candidates
    .split(/^### /mu)
    .slice(1)
    .map(section => {
      const id = /^`([^`]+)`\n/u.exec(section)?.[1];
      if (id === undefined) throw new Error(`packs/${pack}/README.md: the heading \`### ${section.split("\n", 1)[0]}\` names no rule id`);
      const block = (side: "Caught" | "Passes"): Example => {
        const match = new RegExp(`^${side} \\(\`([^\`]+)\`\\):\\n\\n\`\`\`\\w+\\n([\\s\\S]*?)^\`\`\`$`, "mu").exec(section);
        if (match === null) throw new Error(`packs/${pack}/README.md: ${id} has no ${side} block`);
        return { path: match[1]!, text: match[2]! };
      };
      return { id, caught: block("Caught"), passes: block("Passes") };
    });
}

describe.each(PACKS)("pack %s candidate example pairs", pack => {
  it.each(readmePairs(pack))("$id: the caught example fires, the passes example does not", ({ id, caught, passes }) => {
    if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
    const text = readFileSync(repositoryPath("packs", pack, "candidates", "rules", `${id}.json`), "utf8");
    const codes = new Set(documentSchema.parse(JSON.parse(text)).clauses.flatMap(clause => (clause.violation === undefined ? [] : [clause.violation.code])));
    const compiled = runtime.compile(text, "json", capabilities);
    for (const example of [caught, passes]) expect(compiled.inScope(example.path), `${example.path} is in ${id}'s scope`).toBe(true);
    const caughtResult = evaluateExample(compiled, caught.path, caught.text);
    const passesResult = evaluateExample(compiled, passes.path, passes.text);
    expect({ caught: caughtResult.failures, passes: passesResult.failures }).toEqual({ caught: [], passes: [] });
    expect(caughtResult.codes.filter(code => codes.has(code))).not.toEqual([]);
    expect(passesResult.codes.filter(code => codes.has(code))).toEqual([]);
  });
});
