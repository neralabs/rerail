import { describe, expect, it } from "vitest";
import { DatabaseSync } from "node:sqlite";
import { holdsTable, tableColumns } from "../../../src/engine/storage/table-probe.js";

describe("table probes", () => {
  it("say which tables and columns a file holds", () => {
    using database = new DatabaseSync(":memory:");
    database.exec("CREATE TABLE present (a, b); CREATE VIEW seen AS SELECT a FROM present;");

    expect(holdsTable(database, "present")).toBe(true);
    expect(holdsTable(database, "seen")).toBe(false);
    expect(holdsTable(database, "absent")).toBe(false);
    expect(tableColumns(database, "present")).toEqual(["a", "b"]);
    expect(tableColumns(database, "absent")).toEqual([]);
  });
});
