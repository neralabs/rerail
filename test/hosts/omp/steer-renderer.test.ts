import type { ExtensionAPI, Theme } from "@oh-my-pi/pi-coding-agent";
import { describe, expect, it } from "vitest";
import { RULE_MESSAGE_TYPE, linkSteers, steerBlock } from "../../../src/engine/index.js";
import { formatOmpSteer, registerOmpSteerRenderer, type SteerTheme } from "../../../src/hosts/omp/steer-renderer.js";
import { fixturesPath } from "../../support/oracles.js";

const DASHBOARD = "http://127.0.0.1:4737/?repository=acme-web";
const WIDTH = 100;

/** A TypeScript-pack steer as the code-rule enforcer renders it, its `Why?` link made absolute as the OMP adapter does. */
function codeRuleSteer(...rules: Array<{ id: string; guidance: string }>): string {
  return linkSteers(
    rules.map(({ id, guidance }) => steerBlock(id, guidance, { mode: "steer", origin: "From the TypeScript rule pack." })).join("\n\n"),
    DASHBOARD,
  );
}

const NO_EXPLICIT_ANY = {
  id: "typescript-eslint-no-explicit-any",
  guidance: "Use unknown or a real type in exported signatures. (web/src/features/orders/useOrders.ts:1:33)",
};
const NO_FLOATING_PROMISES = {
  id: "typescript-eslint-no-floating-promises",
  guidance: "Await the promise, return it, or mark it with void. (web/src/features/orders/useOrders.ts:12:5)",
};

/** Tags each colour so the goldens read the structure rather than escape codes. */
const theme: SteerTheme = { fg: (colour, text) => `<${colour}>${text}</${colour}>` };
/** The coloured `R E R A I L`, tagged like the theme's colours. */
const MARK = "<mark>R E R A I L</mark>";

/** The host's components, reduced to what a golden needs: each row padded on the left as the host pads it. */
class Text {
  constructor(readonly text: string, readonly paddingX: number, readonly paddingY: number) {}
  render(_width: number): string[] {
    return [`${" ".repeat(this.paddingX)}${this.text}`];
  }
}
class Container {
  readonly children: Text[] = [];
  addChild(child: Text): void {
    this.children.push(child);
  }
  render(width: number): string[] {
    return this.children.flatMap(child => child.render(width));
  }
}

type Renderer = (message: { customType: string; content: unknown }, options: { expanded: boolean }, theme: Theme) => unknown;

function registered(components: { Container?: unknown; Text?: unknown } | undefined, hyperlinks: boolean): Renderer {
  let renderer: Renderer | undefined;
  registerOmpSteerRenderer({
    registerMessageRenderer: (customType: string, render: Renderer) => {
      expect(customType).toBe(RULE_MESSAGE_TYPE);
      renderer = render;
    },
    pi: components,
  } as unknown as Pick<ExtensionAPI, "registerMessageRenderer"> & { pi?: Partial<Pick<ExtensionAPI["pi"], "Container" | "Text">> }, () => hyperlinks, () => MARK);
  return renderer!;
}

/** What OMP draws for `content`, with each OSC 8 link written as a tag so the golden shows where it lands. */
function drawn(content: string, expanded: boolean, hyperlinks: boolean): string {
  const steer = registered({ Container, Text }, hyperlinks)({ customType: RULE_MESSAGE_TYPE, content }, { expanded }, theme as Theme);
  expect(steer).toBeInstanceOf(Container);
  return `${(steer as Container).render(WIDTH).join("\n")}\n`
    .replace(/\u001b\]8;;([^\u0007]+)\u0007(.*?)\u001b\]8;;\u0007/gu, "<link $1>$2</link>");
}

describe("OMP steer renderer", () => {
  it.each([
    { scene: "one-rule", content: codeRuleSteer(NO_EXPLICIT_ANY) },
    { scene: "two-rules", content: codeRuleSteer(NO_EXPLICIT_ANY, NO_FLOATING_PROMISES) },
  ])("draws $scene collapsed and expanded, with and without terminal links", async ({ scene, content }) => {
    for (const hyperlinks of [true, false]) {
      const variant = hyperlinks ? scene : `${scene}.no-hyperlinks`;
      await expect(drawn(content, false, hyperlinks)).toMatchFileSnapshot(fixturesPath("host", "session-enforcement", "omp", `${variant}.collapsed.txt`));
      await expect(drawn(content, true, hyperlinks)).toMatchFileSnapshot(fixturesPath("host", "session-enforcement", "omp", `${variant}.expanded.txt`));
    }
  });

  it("leaves the why? off a steer whose link never became absolute", () => {
    const rows = formatOmpSteer(steerBlock("checkout", "Check checkout requirements.", { mode: "steer" }), { expanded: true, hyperlinks: true, mark: MARK }, theme);
    expect(rows.map(row => row.text)).toEqual([
      "<accent>◆</accent> <mark>R E R A I L</mark><dim> · </dim>checkout  <accent>steered</accent>",
      "<dim>Set to Steer: fix it, or tell the person in one line why you won't.</dim>",
      "<dim>Check checkout requirements.</dim>",
    ]);
  });

  it("strips terminal controls from the guidance a repository wrote", () => {
    const content = codeRuleSteer({ id: "local:shout", guidance: "Stop\u001b[2J here.\u0007" });
    const rows = formatOmpSteer(content, { expanded: true, hyperlinks: false, mark: MARK }, theme);
    expect(rows.map(row => row.text)).toContain("<dim>Stop[2J here.</dim>");
    expect(rows.every(row => !/[\u0000-\u001f]/u.test(row.text))).toBe(true);
  });

  it("falls back to OMP's default card without the host's components or a Rerail headline", () => {
    const content = codeRuleSteer(NO_EXPLICIT_ANY);
    const message = { customType: RULE_MESSAGE_TYPE, content };
    expect(registered(undefined, true)(message, { expanded: false }, theme as Theme)).toBeUndefined();
    expect(registered({ Text }, true)(message, { expanded: false }, theme as Theme)).toBeUndefined();
    expect(registered({ Container, Text }, true)({ ...message, content: "Plain guidance." }, { expanded: false }, theme as Theme)).toBeUndefined();
  });

  it("draws text content blocks as OMP stores them", () => {
    const content = [{ type: "text", text: codeRuleSteer(NO_EXPLICIT_ANY) }];
    const steer = registered({ Container, Text }, true)({ customType: RULE_MESSAGE_TYPE, content }, { expanded: false }, theme as Theme);
    expect((steer as Container).children).toHaveLength(1);
  });
});
