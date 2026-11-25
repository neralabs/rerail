import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope } from "../../ui/poll.js";
import { actions, patchProblem, stripSignature } from "../../ui/views/merge.js";
import { sampleStatus } from "../support/merge-assist/status.js";
import type * as Shell from "../../ui/shell.js";

const { toast } = vi.hoisted(() => ({ toast: vi.fn() }));
// The page's toast, and no drawing: the writes are driven without a page.
vi.mock("../../ui/shell.js", async importOriginal => ({
  ...(await importOriginal<typeof Shell>()), invalidate: () => {}, renderNow: () => {}, toast,
}));

/** An element as `patchProblem` patches it, noting each text change made while it was hidden. */
class StubElement {
  hidden = false;
  textWhileHidden: string[] = [];
  #text = "";
  readonly #attrs = new Map<string, string>();

  get textContent(): string { return this.#text; }
  set textContent(text: string) {
    if (this.hidden) this.textWhileHidden.push(text);
    this.#text = text;
  }

  getAttribute(name: string): string | null { return this.#attrs.get(name) ?? null; }
  hasAttribute(name: string): boolean { return this.#attrs.has(name); }
  setAttribute(name: string, value: string): void { this.#attrs.set(name, value); }
  removeAttribute(name: string): void { this.#attrs.delete(name); }
}

/** The Settings form's problem paragraph and its two fields, found as `patchProblem` looks them up. */
function settingsForm() {
  const problem = new StubElement(), hook = new StubElement(), timeout = new StubElement();
  const found: Record<string, StubElement> = {
    'p[id="ma-settings-problem"]': problem, 'input[id="ma-hook"]': hook, 'input[id="ma-timeout"]': timeout,
  };
  return { form: { querySelector: (selector: string) => found[selector] ?? null }, problem, hook, timeout };
}

describe("the Settings form's problem", () => {
  it("stays rendered, empty without a problem, so a screen reader announces the text a refused Save puts in it", () => {
    const { form, problem, timeout } = settingsForm();

    patchProblem(form, { problem: null, field: null });
    expect(problem).toMatchObject({ hidden: false, textContent: "" });

    patchProblem(form, { problem: "The timeout must be a whole number of seconds from 1 to 60.", field: "timeout" });
    expect(problem).toMatchObject({ hidden: false, textContent: "The timeout must be a whole number of seconds from 1 to 60." });
    expect(timeout.getAttribute("aria-invalid")).toBe("true");

    patchProblem(form, { problem: null, field: null });
    expect(problem).toMatchObject({ hidden: false, textContent: "" });
    expect(timeout.hasAttribute("aria-invalid")).toBe(false);
    expect(problem.textWhileHidden).toEqual([]);
  });
});

describe("the status strip", () => {
  const github = (fields: object) => sampleStatus("/work/a", { readiness: { github: { ...sampleStatus("/work/a").readiness.github, ...fields } } });

  it("is not rebuilt by a status whose gh reading differs only in when it was read and when it is read again", () => {
    const shown = stripSignature(github({ readAgainInMs: 240_000 }));

    // Scan now and Turn off keep their focus, and a click across the read still lands.
    expect(stripSignature(github({ readAgainInMs: 239_000 }))).toBe(shown);
    expect(stripSignature(github({ readAt: "2026-11-03T14:10:00Z", readAgainInMs: 300_000 }))).toBe(shown);
  });

  it("is rebuilt when what it draws of gh's reading changes", () => {
    const shown = stripSignature(github({}));

    expect(stripSignature(github({ state: "signed-out" }))).not.toBe(shown);
    expect(stripSignature(github({ login: "jdoe" }))).not.toBe(shown);
    expect(stripSignature(github({ readAt: null }))).not.toBe(shown);
  });
});

describe("a Merge Assist write's toast", () => {
  let answer: { status: number; body: unknown };

  beforeEach(() => {
    toast.mockReset();
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("document", { activeElement: null, body: null });
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify(answer.body), { status: answer.status }));
    changeScope({ repository: "A", checkout: "/work/a" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("is the daemon's words alone for a change it made but could not confirm on the disk", async () => {
    answer = { status: 503, body: { error: "not-on-disk", message: "the change was made, but Rerail could not confirm it is on the disk (EIO)" } };
    await actions.maon();

    expect(toast).toHaveBeenCalledExactlyOnceWith("The change was made, but Rerail could not confirm it is on the disk (EIO).");
  });

  it("says what could not be done, then the API's reason, for a refused write", async () => {
    answer = { status: 409, body: { error: "conflict", message: "Merge Assist is already on" } };
    await actions.maon();

    expect(toast).toHaveBeenCalledExactlyOnceWith("Couldn’t turn on Merge Assist. Merge Assist is already on.");
  });
});
