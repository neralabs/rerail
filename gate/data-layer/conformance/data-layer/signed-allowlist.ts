/** Host verification for the pinned escape-hatch reporter's signed configuration. */
import { createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import { z } from "zod";
import type { CodeRule } from "../../src/engine/index.js";
import { fixturesPath } from "../../test/support/oracles.js";

const signature = z.object({ public_key_hex: z.string(), signature: z.string() }).parse(JSON.parse(
  readFileSync(fixturesPath("host", "data-layer-port", "escape-hatch-allowlist-signature.json"), "utf8"),
));

export function escapeHatchSignatureFailure(policy: CodeRule, signatureOverride?: string) {
  const definition = policy.clauses.find(clause => clause.define?.options?.ALLOWLISTED_CLASSES !== undefined)?.define;
  const names = z.array(z.string()).parse(definition?.config?.ALLOWLISTED_CLASSES ?? definition?.options?.ALLOWLISTED_CLASSES?.default);
  // Python json.dumps(sort_keys=True) uses spaces and ASCII escapes in its signed message.
  const allowlist = [...new Set(names)].sort().map(name => JSON.stringify(name).replace(/[\u007f-\uffff]/g,
    character => `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`)).join(", ");
  const message = Buffer.from(`{"allowlist": [${allowlist}], "version": 1}`, "utf8");
  let valid = false;
  try {
    const key = createPublicKey({ format: "der", type: "spki", key: Buffer.concat([
      Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(signature.public_key_hex, "hex"),
    ]) });
    valid = verify(null, message, key, Buffer.from(signatureOverride ?? signature.signature, "base64"));
  } catch { /* Invalid key or signature has the same reporter result as a failed verification. */ }
  if (valid) return undefined;
  return { violations: [{ code: "invalid_allowlist_signature", path: "scripts/lint_sql_grammar_escape_hatches.py",
    message: "SQL grammar escape hatch allowlist signature verification failed", clauseId: "allowlist-signature",
    span: { startByte: 0, startLine: 0, startColumn: 0 } }], failures: [] };
}
