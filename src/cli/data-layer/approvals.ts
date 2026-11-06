/** Public-key verification of schema approval configuration, before SQL reporting. */
import { createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import { decodePythonBase64 } from "./base64.js";

export const schemaApprovalsSchema = z.object({ entries: z.array(z.string()), signature: z.string() }).strict();
export type SchemaApprovals = z.infer<typeof schemaApprovalsSchema>;

function compareCodePoints(left: string, right: string): number {
  const a = [...left], b = [...right];
  for (let index = 0; index < Math.min(a.length, b.length); index++) {
    const difference = a[index]!.codePointAt(0)! - b[index]!.codePointAt(0)!;
    if (difference) return difference;
  }
  return a.length - b.length;
}
const asciiJson = (value: unknown): string => JSON.stringify(value).replace(/[\u007f-\uffff]/g,
  character => `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`);

export function schemaApprovalMessage(entries: readonly string[]): Buffer {
  return Buffer.from(asciiJson({ allowlist: [...entries].sort(compareCodePoints), linter: "schema_introspection_data_paths", version: 1 }), "utf8");
}

export function schemaApprovalFacts(configuration: SchemaApprovals, packDirectory: string) {
  const { entries, signature } = schemaApprovalsSchema.parse(configuration);
  let signatureVerified = !entries.length;
  if (entries.length && !["", "NEEDS_SIGNING", "UNSIGNED"].includes(signature)) {
    try {
      const trust = z.object({ publicKeyHex: z.string().regex(/^[0-9a-fA-F]{64}$/) }).strict().parse(
        JSON.parse(readFileSync(join(packDirectory, "runtime/signing-public-key.json"), "utf8")));
      const key = createPublicKey({ format: "der", type: "spki", key: Buffer.concat([
        Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(trust.publicKeyHex, "hex"),
      ]) });
      signatureVerified = verify(null, schemaApprovalMessage(entries), key, decodePythonBase64(signature));
    } catch { signatureVerified = false; }
  }
  return { entries: [...entries].sort(compareCodePoints), signatureVerified };
}
