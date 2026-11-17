import { describe, expect, it } from "vitest";
import { patchProblem } from "../../ui/views/merge/settings.js";

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
