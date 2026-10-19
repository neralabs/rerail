/**
 * next-no-img-element's message must say what switching to `next/image` takes (NER-1946).
 *
 * Upstream's message (@next/eslint-plugin-next no-img-element) only says to consider `<Image />`. For a remote image,
 * such as a GitHub avatar, the switch also needs `width` and `height` (or `fill`) and a matching
 * `images.remotePatterns` entry; without them `next/image` throws. Agents given upstream's text often kept `<img>`
 * after the steer. The message now names both requirements, and that an `eslint-disable-next-line` with a reason is the
 * way to keep `<img>` for images from arbitrary hosts. What the rule reports is unchanged; this test also checks that
 * the suppression forms the message names do suppress it.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { loadBundledRules } from "../../../../src/engine/catalog/bundled-rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = loadBundledRules([repositoryPath("packs", "next")]).find(candidate => candidate.rule.id === "next-no-img-element");
  if (document === undefined) throw new Error("next-no-img-element is not in packs/next");
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

const compiled = compile();

/** The 1-based lines the rule reports in `source`, each with its message. */
function findings(source: string): { line: number; message: string }[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "tsx", path: "components/avatar.tsx", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => ({ line: violation.span.startLine + 1, message: violation.message })).sort((a, b) => a.line - b.line);
}

const tsx = (...lines: string[]) => [...lines, ""].join("\n");

const AVATAR = tsx(
  "export function Avatar({ login }: { login: string }) {",
  "  return (",
  "    <div className=\"avatar\">",
  "      <img src={`https://github.com/${login}.png`} alt={login} />",
  "    </div>",
  "  );",
  "}",
);

describe("next-no-img-element", () => {
  it("still reports a remote <img>, as upstream does", () => {
    expect(findings(AVATAR).map(finding => finding.line)).toEqual([4]);
  });

  it("says what next/image needs and how to keep <img> for images from arbitrary hosts", () => {
    const [finding] = findings(AVATAR);
    expect(finding?.message).toBe(
      "Using `<img>` could result in slower LCP and higher bandwidth. Consider using `<Image />` from `next/image` or a custom image loader to automatically optimize images. This may incur additional usage or cost from your provider. `<Image />` needs `width` and `height` (or `fill`) unless `src` is a static import, and a remote `src` needs a matching `images.remotePatterns` entry in next.config. To keep `<img>`, e.g. for images from arbitrary hosts, put `// eslint-disable-next-line @next/next/no-img-element -- <reason>` on the line above (as `{/* ... */}` among JSX children). See: https://nextjs.org/docs/messages/no-img-element",
    );
  });

  it.each([
    [
      "a JSX comment with a reason",
      tsx(
        "export function Avatar({ login }: { login: string }) {",
        "  return (",
        "    <div className=\"avatar\">",
        "      {/* eslint-disable-next-line @next/next/no-img-element -- user-supplied image URLs from any host */}",
        "      <img src={`https://github.com/${login}.png`} alt={login} />",
        "    </div>",
        "  );",
        "}",
      ),
    ],
    [
      "a line comment with a reason",
      tsx(
        "export function Avatar({ login }: { login: string }) {",
        "  // eslint-disable-next-line @next/next/no-img-element -- user-supplied image URLs from any host",
        "  return <img src={`https://github.com/${login}.png`} alt={login} />;",
        "}",
      ),
    ],
  ])("is suppressed by the comment the message names: %s", (_name, source) => {
    expect(findings(source)).toEqual([]);
  });
});
