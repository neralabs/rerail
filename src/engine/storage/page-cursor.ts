/**
 * Keyset cursors for newest-first history pages. A cursor names the last row a page returned (its sort key), so the
 * next page starts right after it at index cost, wherever it lies, and rows added meanwhile never shift it. It is
 * opaque to callers: base64url JSON of the sort key.
 */
import { z } from "zod";

/** A cursor that is not one this daemon minted for the listing it was passed to. */
export class InvalidCursorError extends Error {
  constructor(message = "cursor is not a page cursor from this listing") {
    super(message);
    this.name = "InvalidCursorError";
  }
}

export function encodeCursor(key: readonly (string | number)[]): string {
  return Buffer.from(JSON.stringify(key)).toString("base64url");
}

/** The sort key `cursor` encodes, checked against `schema`; throws `InvalidCursorError` otherwise. */
export function decodeCursor<T>(cursor: string, schema: z.ZodType<T>): T {
  let value: unknown;
  try {
    value = JSON.parse(Buffer.from(cursor, "base64url").toString("utf8"));
  } catch {
    throw new InvalidCursorError();
  }
  const parsed = schema.safeParse(value);
  if (!parsed.success) throw new InvalidCursorError();
  return parsed.data;
}
