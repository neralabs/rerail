import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MergeAssistStatus } from "../../src/engine/index.js";
import { changeScope } from "../../ui/poll.js";
import { data, ui } from "../../ui/state.js";
import { onInput, render } from "../../ui/views/merge.js";
import type { FakeElement } from "./support/fake-dom.js";
import { conflict } from "./support/merge-conflicts.js";
import { mergePage, storefrontStatus } from "./support/merge-page.js";

/** Off, with no agent CLI found and gh signed out. */
const fixture = (over: Partial<MergeAssistStatus> = {}): MergeAssistStatus => storefrontStatus({
  enabled: false,
  readiness: {
    judge: { available: false, runner: null, problem: "Neither claude nor codex is on PATH." },
    github: { state: "signed-out", login: null, repository: null },
  },
  lastScan: null,
  ...over,
});

/** On, signed in, scanned two minutes before noon, with one open conflict. */
const onFixture = (): MergeAssistStatus => storefrontStatus({ conflicts: [conflict("conflict:a")] });

describe("the Merge Assist page", () => {
  const page = mergePage();
  const { button, click, toastText, writes } = page;
  const conflicts = () => page.root.querySelector('section[data-section="conflicts"]')!;

  it("pitches catching collisions with work in flight", () => {
    data.mergeAssist = fixture();
    render(page.root);
    expect(page.root.querySelector("p.pitch")!.textContent).toBe(
      "Rerail watches the branches and pull requests in flight and tells you and your coding agent when your work collides with someone else’s, before it merges.");
  });

  describe("while off", () => {
    beforeEach(() => {
      data.mergeAssist = fixture();
      render(page.root);
    });

    it("says what turning it on does and what Rerail found, with the one button", () => {
      const card = page.root.querySelector(".mconsent")!;
      expect(card.textContent).toContain("What turning it on does");
      const bullets = card.querySelectorAll("ul.mbul li").map(li => li.textContent);
      expect(bullets).toHaveLength(3);
      expect(bullets[0]).toBe("Rerail reads your other local branches, and open pull requests through gh with your sign-in, read-only.");
      expect(bullets[1]).toContain("for a one-line summary of each conflict");
      const checks = card.querySelectorAll("ul.mchecks li");
      expect(checks.map(li => [li.dataset.check, li.className])).toEqual([["judge", "mnotready"], ["github", "mnotready"]]);
      expect(checks[0]!.textContent).toContain("need Claude Code or Codex on your PATH");
      expect(button("maon")!.textContent).toBe("Turn on Merge Assist");
      expect(page.root.querySelector(".mstrip")).toBeNull();
      expect(conflicts().hidden).toBe(true);
    });

    it("turning on sends the setting for the checkout the page shows, holds the button, then shows the page on", async () => {
      const turning = click(button("maon"));
      render(page.root);
      expect(button("maon")!.disabled).toBe(true);
      expect(button("maon")!.textContent).toBe("Turning on…");
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]).toMatchObject({
        method: "PUT", url: expect.stringMatching(/\/api\/merge-assist\/settings$/u),
        body: { repository: "A", checkout: "/work/a", enabled: true },
      });

      writes()[0]!.answer(200, { ...fixture(), enabled: true });
      await turning;
      render(page.root);

      expect(toastText()).toBe("Merge Assist is on in storefront. You and your coding agent are told when your work here collides with work in flight.");
      expect(page.root.querySelector(".mconsent")).toBeNull();
      expect(page.root.querySelector(".mstrip")!.textContent).toContain("Not scanned yet");
      expect(conflicts().hidden).toBe(false);
      expect(conflicts().textContent).toContain("No open conflicts");
      expect(conflicts().textContent).toContain("Rerail looks for conflicts on every scan and after your coding agent’s edits");
    });

    it("says why when turning on fails, and offers the button again", async () => {
      const turning = click(button("maon"));
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      writes()[0]!.answer(403, { error: "agent-not-permitted", message: "Only a person can turn Merge Assist on." });
      await turning;
      render(page.root);

      expect(toastText()).toContain("Couldn’t turn on Merge Assist. Only a person can turn Merge Assist on.");
      expect(button("maon")!.disabled).toBe(false);
      expect(data.mergeAssist!.enabled).toBe(false);
    });
  });

  describe("while on", () => {
    beforeEach(() => {
      data.mergeAssist = onFixture();
      render(page.root);
    });

    it("shows the strip, then the open conflicts with their count", () => {
      const strip = page.root.querySelector(".mstrip")!;
      expect(strip.textContent).toContain("Signed in as tbelanger");
      expect(strip.textContent).toContain("Claude Code");
      expect(strip.textContent).toContain("0 of 40 this hour");
      expect(conflicts().querySelector("h2")!.textContent).toBe("Open conflicts1");
      expect(conflicts().querySelectorAll("article.mcard")).toHaveLength(1);
    });

    it("Scan now posts a scan for the checkout and says how many conflicts it found that were not shown before", async () => {
      const scanning = click(button("mascan"));
      render(page.root);
      expect(button("mascan")!.textContent).toBe("Scanning…");
      expect(button("mascan")!.disabled).toBe(true);
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]).toMatchObject({ method: "POST", url: expect.stringMatching(/\/api\/merge-assist\/scan$/u), body: { repository: "A", checkout: "/work/a" } });
      writes()[0]!.answer(200, { ...onFixture(), conflicts: [conflict("conflict:a"), conflict("conflict:b")] });
      await scanning;
      render(page.root);

      expect(toastText()).toBe("Scanned 2 local branches and 1 open pull request. New: 1 conflict.");
      expect(conflicts().querySelectorAll("article.mcard")).toHaveLength(2);
    });

    it("says when another scan holds the checkout, and shows the last results", async () => {
      const scanning = click(button("mascan"));
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      writes()[0]!.answer(200, { ...onFixture(), scanning: true });
      await scanning;
      render(page.root);

      expect(toastText()).toBe("Another scan is running; showing the last results.");
      expect(button("mascan")!.disabled).toBe(true);
    });

    it("keeps the focused button through Scan now and Turn off: it is updated in place, not rebuilt", async () => {
      const scanButton = button("mascan"), offButton = button("maoff");
      const scanning = click(scanButton);
      render(page.root);
      expect(button("mascan")).toBe(scanButton);
      expect(scanButton!.disabled).toBe(true);
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      writes()[0]!.answer(200, onFixture());
      await scanning;
      render(page.root);
      expect(button("mascan")).toBe(scanButton);
      expect([scanButton!.disabled, scanButton!.textContent]).toEqual([false, "Scan now"]);

      const turningOff = click(offButton);
      render(page.root);
      expect(button("maoff")).toBe(offButton);
      expect([offButton!.disabled, offButton!.textContent]).toEqual([true, "Turning off…"]);
      expect(button("madismiss", "conflict:a")!.disabled).toBe(true);
      await vi.waitFor(() => expect(writes()).toHaveLength(2));
      writes()[1]!.answer(500, { error: "internal", message: "Disk full." });
      await turningOff;
      render(page.root);
      expect(button("maoff")).toBe(offButton);
      expect(offButton!.disabled).toBe(false);
      expect(button("madismiss", "conflict:a")!.disabled).toBe(false);
    });

    it("turning off says Rerail no longer looks for conflicts here, and shows the consent card again", async () => {
      const turningOff = click(button("maoff"));
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", enabled: false });
      writes()[0]!.answer(200, { ...onFixture(), enabled: false });
      await turningOff;
      render(page.root);
      expect(toastText()).toBe("Merge Assist is off in storefront. Rerail no longer looks for conflicts here.");
      expect(page.root.querySelector(".mconsent")).not.toBeNull();
      expect(conflicts().hidden).toBe(true);
    });

    it("unfolds what the last scan could not read, and keeps it open across renders", () => {
      data.mergeAssist = { ...onFixture(), lastScan: { at: "2026-11-02T11:58:00Z", peers: { branches: 2, pullRequests: 1 }, problems: ["gh: rate limited"] } };
      render(page.root);
      const fold = page.root.querySelector("details.mproblems")!;
      expect(fold.querySelector("summary")!.textContent).toBe("1 thing the last scan could not read");
      click(fold.querySelector("summary"));
      render(page.root);
      expect(page.root.querySelector("details.mproblems")!.hasAttribute("open")).toBe(true);
      expect(ui.mergeOpen.has("problems")).toBe(true);
    });

    it("shows why it cannot read Merge Assist and nothing to act on, not the last status", () => {
      data.mergeAssistError = "/api/merge-assist?repository=A answered 500";
      render(page.root);
      expect(page.root.querySelector('[role="alert"]')!.textContent).toContain("Couldn’t read Merge Assist for this checkout.");
      expect(page.root.querySelectorAll("button[data-act]").filter(b => b.closest("[hidden]") === null).map(b => b.dataset.act)).toEqual([]);
      expect(conflicts().hidden).toBe(true);
      expect(page.root.querySelector(".mstrip")).toBeNull();

      data.mergeAssistError = null;
      render(page.root);
      expect(page.root.querySelector('[role="alert"]')).toBeNull();
      expect(button("mascan")!.disabled).toBe(false);
    });

    it("forgets a scan or a switch in flight when the page moves to another repository", async () => {
      const scanning = click(button("mascan"));
      const turningOff = click(button("maoff"));
      await vi.waitFor(() => expect(writes()).toHaveLength(2));
      changeScope({ repository: "B", checkout: "/work/b" });
      data.repository = { setUp: true, repositoryKey: "B", name: "billing" };
      data.mergeAssist = { ...onFixture(), checkout: "/work/b" };
      render(page.root);
      expect([button("mascan")!.disabled, button("mascan")!.textContent]).toEqual([false, "Scan now"]);
      expect(button("maoff")!.disabled).toBe(false);

      // The first repository's answers land nowhere, and leave the second's buttons alone.
      writes()[0]!.answer(200, onFixture());
      writes()[1]!.answer(200, { ...onFixture(), enabled: false });
      await Promise.all([scanning, turningOff]);
      render(page.root);
      expect(data.mergeAssist!.checkout).toBe("/work/b");
      expect([ui.mergeScanning, ui.mergeSwitching]).toEqual([false, false]);
    });

    describe("the Settings section", () => {
      const settings = () => page.root.querySelector("details.msettings")!;
      const input = (id: string) => page.root.querySelector(`input[id="${id}"]`) as (FakeElement & { value: string }) | null;
      const type = (id: string, text: string) => {
        const el = input(id)!;
        el.value = text;
        onInput(el);
        render(page.root);
      };
      const checked = () => settings().querySelectorAll("button.mopt").filter(b => b.getAttribute("aria-checked") === "true").map(b => b.dataset.i);
      const problem = () => settings().querySelector('p[id="ma-settings-problem"]')!;
      const saved = { delivery: "terminal" as const, hook: { command: ["./scripts/on-conflict.sh", "--quiet"], timeoutSeconds: 20 } };

      it("shows the saved delivery mode, hook command and timeout, with what each mode does, folded until opened", () => {
        data.mergeAssist = { ...onFixture(), settings: saved };
        render(page.root);
        expect(settings().hidden).toBe(false);
        expect(settings().hasAttribute("open")).toBe(false);
        expect(settings().querySelector("summary")!.textContent).toBe("Settings: delivery terminal, conflict hook: ./scripts/on-conflict.sh");
        click(settings().querySelector("summary"));
        render(page.root);
        expect(settings().hasAttribute("open")).toBe(true);
        expect(settings().querySelectorAll("button.mopt").map(b => [b.dataset.i, b.querySelector(".mopt-l")!.textContent])).toEqual([["chat", "Chat"], ["terminal", "Terminal"], ["both", "Both"]]);
        expect(settings().querySelectorAll("button.mopt .sub").every(line => line.textContent.length > 0)).toBe(true);
        expect(checked()).toEqual(["terminal"]);
        expect(input("ma-hook")!.value).toBe("[\"./scripts/on-conflict.sh\",\"--quiet\"]");
        expect(input("ma-timeout")!.value).toBe("20");
        expect(problem().hidden).toBe(true);
      });

      it("Save sends the delivery and the hook command for the checkout, then shows the saved values", async () => {
        expect(checked()).toEqual(["both"]);
        expect([input("ma-hook")!.value, input("ma-timeout")!.value]).toEqual(["", "10"]);
        click(button("madelivery", "terminal"));
        render(page.root);
        expect(checked()).toEqual(["terminal"]);
        const field = input("ma-hook");
        type("ma-hook", " [\"./scripts/on-conflict.sh\", \"--quiet\"] ");
        type("ma-timeout", "20");
        expect(input("ma-hook")).toBe(field); // typing never rebuilds the form

        const saving = click(button("masettings"));
        render(page.root);
        expect([button("masettings")!.disabled, button("masettings")!.textContent]).toEqual([true, "Saving…"]);
        await vi.waitFor(() => expect(writes()).toHaveLength(1));
        expect(writes()[0]).toMatchObject({ method: "PUT", url: expect.stringMatching(/\/api\/merge-assist\/settings$/u) });
        expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", delivery: "terminal", hook: saved.hook });

        writes()[0]!.answer(200, { ...onFixture(), settings: saved });
        await saving;
        render(page.root);
        expect(toastText()).toBe("Saved Merge Assist’s settings for storefront.");
        expect(ui.mergeSettings).toBeNull();
        expect(settings().querySelector("summary")!.textContent).toBe("Settings: delivery terminal, conflict hook: ./scripts/on-conflict.sh");
        expect([input("ma-hook")!.value, input("ma-timeout")!.value]).toEqual(["[\"./scripts/on-conflict.sh\",\"--quiet\"]", "20"]);
        expect([button("masettings")!.disabled, button("masettings")!.textContent]).toEqual([false, "Save"]);
      });

      it("an empty command removes the hook", async () => {
        data.mergeAssist = { ...onFixture(), settings: saved };
        render(page.root);
        type("ma-hook", "");
        const saving = click(button("masettings"));
        await vi.waitFor(() => expect(writes()).toHaveLength(1));
        expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", delivery: "terminal", hook: null });
        writes()[0]!.answer(200, onFixture());
        await saving;
      });

      it("refuses a hook command that is not a JSON array of non-empty strings, sending nothing, until it is fixed", async () => {
        type("ma-hook", "./scripts/on-conflict.sh --quiet");
        await click(button("masettings"));
        render(page.root);
        expect(writes()).toHaveLength(0);
        expect(problem().hidden).toBe(false);
        expect(problem().textContent).toBe("The hook command must be a JSON array of non-empty strings, like [\"./scripts/on-conflict.sh\", \"--quiet\"].");
        expect(input("ma-hook")!.getAttribute("aria-invalid")).toBe("true");

        type("ma-hook", "[\"./scripts/on-conflict.sh\"]");
        expect(problem().hidden).toBe(true);
        expect(input("ma-hook")!.hasAttribute("aria-invalid")).toBe(false);
        type("ma-timeout", "90");
        await click(button("masettings"));
        render(page.root);
        expect(writes()).toHaveLength(0);
        expect(problem().textContent).toBe("The timeout must be a whole number of seconds from 1 to 60.");
        expect(input("ma-timeout")!.getAttribute("aria-invalid")).toBe("true");
      });

      it("is not shown while off", () => {
        data.mergeAssist = fixture();
        render(page.root);
        expect(settings().hidden).toBe(true);
      });
    });
  });
});
