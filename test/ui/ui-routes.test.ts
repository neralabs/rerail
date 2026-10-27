import { describe, expect, it } from "vitest";
import { bootRoute, hashOf, parseRoute, repositoryUrl, urlFor, type Route } from "../../ui/routes.js";

const PAGE = "http://127.0.0.1:4000/";

describe("a hash", () => {
  it.each([
    ["", { view: "overview" }],
    ["#", { view: "overview" }],
    ["#overview", { view: "overview" }],
    ["#packs", { view: "packs" }],
    ["#packs?review=1", { view: "review" }],
    ["#create", { view: "create" }],
    ["#created", { view: "created" }],
    ["#history", { view: "history" }],
    ["#history?rule=typescript%3Ano-any", { view: "history", filterRule: "typescript:no-any" }],
    ["#sessions", { view: "sessions" }],
    ["#settings", { view: "settings" }],
    ["#pack/typescript", { view: "pack", pack: "typescript" }],
    ["#somewhere-else", { view: "overview" }],
  ])("%s names its view", (hash, route) => {
    expect(parseRoute(hash)).toEqual(route);
  });

  // Steers, the CLI and the status line carry these; they must keep landing where they did.
  it.each([
    ["#rules", { view: "overview" }],
    ["#activity", { view: "history" }],
    ["#activity?rule=lib:db-calls", { view: "history", filterRule: "lib:db-calls" }],
    ["#rule-packs", { view: "packs" }],
    ["#rule-packs?review=1", { view: "review" }],
    ["#rule-packs?review=0", { view: "packs" }],
  ])("from before v8, %s still resolves", (hash, route) => {
    expect(parseRoute(hash)).toEqual(route);
  });

  it("opens a rule link's sheet over whatever view shows, its id taken whole", () => {
    expect(parseRoute("#rule/typescript:no-explicit-any")).toEqual({ view: null, rule: "typescript:no-explicit-any" });
    expect(parseRoute("#rule/lib%3Awhy%3F?x")).toEqual({ view: null, rule: "lib:why??x" });
  });

  it("drops a rule or pack link it cannot decode, rather than opening a wrong id", () => {
    expect(parseRoute("#rule/%E0%A4%A")).toEqual({ view: null });
    expect(parseRoute("#rule/")).toEqual({ view: null });
    expect(parseRoute("#pack/")).toEqual({ view: "overview" });
  });

  it.each<Route>([
    { view: "overview" },
    { view: "pack", pack: "typescript" },
    { view: "pack", pack: "a pack/with?odd#chars" },
    { view: "created" },
    { view: "packs" },
    { view: "review" },
    { view: "create" },
    { view: "history" },
    { view: "history", filterRule: "lib:no db calls?" },
    { view: "sessions" },
    { view: "settings" },
  ])("written back for %o, reads as the same route", route => {
    expect(parseRoute(hashOf(route))).toEqual(route);
  });

  it("is the open rule's link while its sheet shows, over any view", () => {
    expect(hashOf({ view: "history", filterRule: "a", rule: "typescript:no-any" })).toBe("rule/typescript:no-any");
    expect(parseRoute(`#${hashOf({ view: "settings", rule: "lib:a/b c" })}`)).toEqual({ view: null, rule: "lib:a/b c" });
  });
});

describe("a fresh page", () => {
  it("opens a rule link over Overview", () => {
    expect(bootRoute("?repository=A", "#rule/r1")).toEqual({ view: "overview", rule: "r1" });
  });

  it("opens a session link on Sessions whatever the hash says, a rule's sheet over it included", () => {
    expect(bootRoute("?repository=A&sessionId=s1&host=claude-code", "#sessions")).toEqual({ view: "sessions", session: "s1" });
    expect(bootRoute("?sessionId=s1", "#activity")).toEqual({ view: "sessions", session: "s1" });
    expect(bootRoute("?sessionId=s1", "#rule/r1")).toEqual({ view: "sessions", session: "s1", rule: "r1" });
  });

  it("opens the hash's view without a session link", () => {
    expect(bootRoute("?repository=A", "#rule-packs?review=1")).toEqual({ view: "review" });
  });
});

describe("the page's URL", () => {
  it("keeps the repository and checkout scope on every view", () => {
    const at = `${PAGE}?repository=A&checkout=%2Fw%2Ftree#overview`;
    for (const route of [{ view: "history" }, { view: "pack", pack: "zod" }, { view: "settings" }, { view: "overview", rule: "r" }] as const) {
      const url = new URL(urlFor(at, route));
      expect(url.searchParams.get("repository")).toBe("A");
      expect(url.searchParams.get("checkout")).toBe("/w/tree");
      expect(url.hash).toBe(`#${hashOf(route)}`);
    }
  });

  it("names the selected session on Sessions and drops it, with the host it came with, elsewhere", () => {
    const linked = `${PAGE}?repository=A&sessionId=s1&host=codex#sessions`;
    const same = new URL(urlFor(linked, { view: "sessions" }, "s1"));
    expect([same.searchParams.get("sessionId"), same.searchParams.get("host")]).toEqual(["s1", "codex"]);
    const other = new URL(urlFor(linked, { view: "sessions" }, "s2"));
    expect([other.searchParams.get("sessionId"), other.searchParams.get("host")]).toEqual(["s2", null]);
    const away = new URL(urlFor(linked, { view: "history" }));
    expect([away.searchParams.get("sessionId"), away.searchParams.get("host"), away.searchParams.get("repository")]).toEqual([null, null, "A"]);
  });

  it("switching repository drops the worktree and the session and lands on Overview", () => {
    const url = new URL(repositoryUrl(`${PAGE}?repository=A&checkout=%2Fw&sessionId=s1&host=pi#settings`, "B"));
    expect([...url.searchParams]).toEqual([["repository", "B"]]);
    expect(url.hash).toBe("#overview");
    expect([...new URL(repositoryUrl(`${PAGE}?repository=A#history`, null)).searchParams]).toEqual([]);
  });
});
