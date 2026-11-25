/**
 * How a reader takes a JSON list Rerail stores as text in a column, so one row whose text a damaged file or a hand edit
 * left costs that row's list, never the page that holds it: the text is parsed once a row (`storedList`), a text that is
 * no JSON array of the list's items reads as none, and the reader names the row once in its log (`nameOnce`).
 */

/** The JSON type of a stored list's items, as SQLite's `json_each` names it in its `type` column. */
export type StoredItem = "text" | "object";

/** Whether a parsed item is of each `StoredItem` type: a string, or a plain object. */
const IS_ITEM: Record<StoredItem, (item: unknown) => boolean> = {
  text: item => typeof item === "string",
  object: item => typeof item === "object" && item !== null && !Array.isArray(item),
};

/** What a list of each `StoredItem` type holds, in the words a log line or a failure uses. */
export const ITEMS: Record<StoredItem, string> = { text: "strings", object: "objects" };

/** The JSON array of `item`s `text` holds, or why it holds none. */
export function storedList(text: string, item: StoredItem): unknown[] | string {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    return (error as SyntaxError).message; // JSON.parse throws nothing else
  }
  if (!Array.isArray(value)) return "it parses to another JSON value";
  return value.every(IS_ITEM[item]) ? value : "an item of it is another JSON value";
}

/** Where a reader writes the line that names a row whose stored text does not read. */
export type StoredListLog = Pick<NodeJS.WriteStream, "write">;

/**
 * Each place this process has named in a log, by its database file and place, so a page read again, or the same text
 * read through another connection, writes no line again.
 */
const NAMED = new Set<string>();

/** Writes `line` to `log` the first time this process meets `place` in the database `file`, and never again. */
export function nameOnce(log: StoredListLog, { file, place }: { file: string; place: string }, line: string): void {
  const key = `${file}\0${place}`;
  if (NAMED.has(key)) return;
  NAMED.add(key);
  log.write(line);
}
