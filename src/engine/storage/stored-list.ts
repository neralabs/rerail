/**
 * How a reader takes a JSON list Rerail stores as text in a column, so one row whose text a damaged file or a hand edit
 * left costs that row's list, never the page that holds it: the text is parsed once a row (`storedList`), a text that is
 * no JSON array of the list's items reads as none, and the reader names the row once in its log (`nameOnce`). A list
 * whose items each stand alone reads without the items that are not of its type (`storedItems`), so such an item costs
 * itself, never the items beside it.
 */

/** The JSON type of a stored list's items, as SQLite's `json_each` names it in its `type` column. */
export type StoredItem = "text" | "object";

/** Whether a parsed item is of each `StoredItem` type: a string, or a plain object. */
const IS_ITEM: Record<StoredItem, (item: unknown) => boolean> = {
  text: item => typeof item === "string",
  object: item => typeof item === "object" && item !== null && !Array.isArray(item),
};

/** Whether `value`, one parsed item of a stored list, is of JSON type `item`, as `storedList` tests each item. */
export function isStoredItem(value: unknown, item: StoredItem): boolean {
  return IS_ITEM[item](value);
}

/** What a list of each `StoredItem` type holds, in the words a log line or a failure uses. */
export const ITEMS: Record<StoredItem, string> = { text: "strings", object: "objects" };

/** The JSON array `text` holds, or why it holds none. */
function storedArray(text: string): unknown[] | string {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    return (error as SyntaxError).message; // JSON.parse throws nothing else
  }
  return Array.isArray(value) ? value : "it parses to another JSON value";
}

/** The JSON array of `item`s `text` holds, or why it holds none. */
export function storedList(text: string, item: StoredItem): unknown[] | string {
  const value = storedArray(text);
  if (typeof value === "string") return value;
  return value.every(entry => isStoredItem(entry, item)) ? value : "an item of it is another JSON value";
}

/**
 * The `item`s of the JSON array `text` holds and how many of its items are not, which the reader leaves out, or why
 * `text` holds no array. An audit's lists read whole or as none (`storedList`), since its record says which it lost.
 */
export function storedItems(text: string, item: StoredItem): { items: unknown[]; left: number } | string {
  const value = storedArray(text);
  if (typeof value === "string") return value;
  const items = value.filter(entry => isStoredItem(entry, item));
  return { items, left: value.length - items.length };
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
