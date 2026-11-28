import { createHash } from "node:crypto";

export const runtime = "edge";

export function loader() {
  return new Response(createHash("sha256").update("og").digest("hex"));
}
