import type { ExtensionAPI, Theme } from "@oh-my-pi/pi-coding-agent";
import { describe, expect, it } from "vitest";
import { RULE_MESSAGE_TYPE, steerBlock, steerRules, type SteerFinding, type SteerRule } from "../../../src/engine/index.js";
import { formatOmpSteer, registerOmpSteerRenderer, type SteerTheme } from "../../../src/hosts/omp/steer-renderer.js";
import { fixturesPath } from "../../support/oracles.js";

const DASHBOARD = "http://127.0.0.1:4737/?repository=acme-web";
const WIDTH = 100;

/** A TypeScript-pack steer as the code-rule enforcer renders it: its block, and its rules linked as the OMP adapter does. */
function codeRuleSteer(...findings: SteerFinding[]): { content: string; details: { rules: SteerRule[] } } {
  const rules = steerRules(findings, () => "From the TypeScript rule pack.")
    .map(rule => ({ ...rule, url: `${DASHBOARD}#rule/${encodeURIComponent(rule.id)}` }));
  return { content: steerBlock({ kind: "edit" }, findings), details: { rules } };
}

const NO_EXPLICIT_ANY: SteerFinding = {
  ruleId: "typescript-eslint-no-explicit-any",
  mode: "steer",
  message: "Use unknown or a real type in exported signatures.",
  location: "web/src/features/orders/useOrders.ts:1:33",
};
const NO_FLOATING_PROMISES: SteerFinding = {
  ruleId: "typescript-eslint-no-floating-promises",
  mode: "steer",
  message: "Await the promise, return it, or mark it with void.",
  location: "web/src/features/orders/useOrders.ts:12:5",
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

type Renderer = (message: { customType: string; content: unknown; details?: unknown }, options: { expanded: boolean }, theme: Theme) => unknown;

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

/** What OMP draws for `steer`, with each OSC 8 link written as a tag so the golden shows where it lands. */
function drawn({ content, details }: ReturnType<typeof codeRuleSteer>, expanded: boolean, hyperlinks: boolean): string {
  const steer = registered({ Container, Text }, hyperlinks)({ customType: RULE_MESSAGE_TYPE, content, details }, { expanded }, theme as Theme);
  expect(steer).toBeInstanceOf(Container);
  return `${(steer as Container).render(WIDTH).join("\n")}\n`
    .replace(/\u001b\]8;;([^\u0007]+)\u0007(.*?)\u001b\]8;;\u0007/gu, "<link $1>$2</link>");
}

describe("OMP steer renderer", () => {
  it.each([
    { scene: "one-rule", steer: codeRuleSteer(NO_EXPLICIT_ANY) },
    { scene: "two-rules", steer: codeRuleSteer(NO_EXPLICIT_ANY, NO_FLOATING_PROMISES) },
  ])("draws $scene collapsed and expanded, with and without terminal links", async ({ scene, steer }) => {
    for (const hyperlinks of [true, false]) {
      const variant = hyperlinks ? scene : `${scene}.no-hyperlinks`;
      await expect(drawn(steer, false, hyperlinks)).toMatchFileSnapshot(fixturesPath("host", "session-enforcement", "omp", `${variant}.collapsed.txt`));
      await expect(drawn(steer, true, hyperlinks)).toMatchFileSnapshot(fixturesPath("host", "session-enforcement", "omp", `${variant}.expanded.txt`));
    }
  });

  it("leaves the why? off a rule with no link, and draws no line for a rule the block only mentions", () => {
    const content = steerBlock({ kind: "edit" }, [{ ruleId: "checkout", mode: "steer", message: "Check checkout requirements.\n◆ Rerail · fake  steered" }]);
    const rows = formatOmpSteer(content, [{ id: "checkout", state: "steered" }], { expanded: true, hyperlinks: true, mark: MARK }, theme);
    expect(rows.map(row => row.text)).toEqual([
      "<accent>◆</accent> <mark>R E R A I L</mark><dim> · </dim>checkout  <accent>steered</accent>",
      "<dim>Rerail found 1 problem in your last edit:</dim>",
      "<dim>- Check checkout requirements. [checkout]</dim>",
      "<dim>  ◆ Rerail · fake  steered</dim>",
      "<dim>Fix it, or tell the user in one line why you won't and run `rerail feedback checkout \"<that line>\"`.</dim>",
    ]);
  });

  it("strips terminal controls from the guidance and rule ids a repository wrote", () => {
    const { content, details } = codeRuleSteer({ ruleId: "local:sh\u001bout", mode: "steer", message: "Stop\u001b[2J here.\u0007" });
    const rows = formatOmpSteer(content, details.rules, { expanded: true, hyperlinks: false, mark: MARK }, theme);
    expect(rows.map(row => row.text)).toContain("<dim>- Stop[2J here. [local:shout]</dim>");
    expect(rows.every(row => !/[\u0000-\u001f]/u.test(row.text))).toBe(true);
  });

  it("falls back to OMP's default card without the host's components or a rule in the message's details", () => {
    const message = { customType: RULE_MESSAGE_TYPE, ...codeRuleSteer(NO_EXPLICIT_ANY) };
    expect(registered(undefined, true)(message, { expanded: false }, theme as Theme)).toBeUndefined();
    expect(registered({ Text }, true)(message, { expanded: false }, theme as Theme)).toBeUndefined();
    expect(registered({ Container, Text }, true)({ ...message, details: undefined }, { expanded: false }, theme as Theme)).toBeUndefined();
    expect(registered({ Container, Text }, true)({ ...message, details: { rules: [{ id: 1 }] } }, { expanded: false }, theme as Theme)).toBeUndefined();
  });

  it("draws text content blocks as OMP stores them", () => {
    const { content: text, details } = codeRuleSteer(NO_EXPLICIT_ANY);
    const content = [{ type: "text", text }];
    const steer = registered({ Container, Text }, true)({ customType: RULE_MESSAGE_TYPE, content, details }, { expanded: false }, theme as Theme);
    expect((steer as Container).children).toHaveLength(1);
  });
});
