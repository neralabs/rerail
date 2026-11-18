import { expect, test } from "./dashboard-test.js";

/**
 * Something between the browser and the daemon can hold a streamed response until it ends: a proxy that buffers
 * responses, an antivirus web shield, the browser's own proxy for a daemon opened by its network address. The change
 * stream then never brings a message. Playwright's route.fetch() is such a proxy: it hands the page a response only
 * once the whole of it has arrived. The dashboard must still load and keep reading, as it did when it polled.
 */
test("the dashboard loads and keeps reading behind a proxy that holds the change stream until it ends", async ({ page, rerail }) => {
  // Two missed beats, then two reads at the pace: past the default 30 s once the scenario's daemon starts.
  test.setTimeout(60_000);
  const url = await rerail.dashboard("mature");
  await page.route("**/api/changes**", async route => {
    try {
      await route.fulfill({ response: await route.fetch() });
    } catch {
      // The page gave up on the stream and aborted it, or the test ended: nothing reached it.
    }
  });
  let reads = 0;
  page.on("request", request => { if (new URL(request.url()).pathname === "/api/repositories") reads += 1; });

  await page.goto(url);

  // Two missed beats (12 s) tell the page no stream reaches it; it then reads every 5 s.
  await expect(page.locator("#statusText")).toHaveText("Live", { timeout: 20_000 });
  const first = reads;
  await expect.poll(() => reads, { timeout: 15_000 }).toBeGreaterThanOrEqual(first + 2);
});
