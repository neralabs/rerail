import { expect, test } from "./dashboard-test.js";

/**
 * A browser gives one origin six HTTP/1.1 connections, shared by all its tabs, and a tab that follows the daemon's
 * change stream holds one. Six tabs of one browser must each load and show the daemon's answer, whether the browser
 * has Web Locks to count the tabs that hold a stream, or has none (a page that is not a secure context, such as a
 * daemon started with --insecure-http opened by its network address), when no tab holds one.
 */
for (const locks of [true, false]) {
  test(`six tabs of one browser each load the dashboard ${locks ? "with" : "without"} Web Locks`, async ({ context, rerail }) => {
    const url = await rerail.dashboard("mature");
    if (!locks) await context.addInitScript(() => { Object.defineProperty(Navigator.prototype, "locks", { get: () => undefined }); });

    const tabs = [];
    for (let tab = 0; tab < 6; tab += 1) {
      const page = await context.newPage();
      await page.goto(url);
      tabs.push(page);
    }

    // "Connecting…" holds until the page's first read answers: a tab whose reads wait for a connection keeps it.
    for (const page of tabs) await expect(page.locator("#statusText")).not.toHaveText("Connecting…", { timeout: 15_000 });
  });
}
