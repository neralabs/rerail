import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "./dashboard-test.js";
import { startHoldingProxy } from "./support/holding-proxy.js";
import { scenarioDashboardUrl, seedScenarioFolder, startScenarioDaemon } from "./support/scenario-daemon.js";

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

/**
 * Such a proxy passes on what it held when the daemon's connection closes, so a daemon restart hands the page a
 * stream's messages after all. When the daemon is back the page must read again and say Live, as it did when it polled.
 */
test("the dashboard reads again and says Live after a daemon restart behind a proxy that hands on the held stream when the daemon's connection closes", async ({ page, context }) => {
  // Two missed beats and a read, a stop, a start, the retry's pause (up to 30 s) and two reads at the pace.
  test.setTimeout(150_000);
  const root = mkdtempSync(join(tmpdir(), "rerail-e2e-restart-"));
  const manifest = await seedScenarioFolder("no-packs", root);
  let daemon = await startScenarioDaemon(manifest);
  const port = Number(new URL(daemon.origin).port);
  const proxy = await startHoldingProxy(port);
  try {
    await context.addInitScript(([origin, token]) => {
      if (location.origin === origin) localStorage.setItem("rerail-access", token);
    }, [proxy.origin, daemon.accessToken] as const);
    let reads = 0;
    page.on("request", request => { if (new URL(request.url()).pathname === "/api/repositories") reads += 1; });
    const status = page.locator("#statusText");
    await page.goto(scenarioDashboardUrl(proxy.origin, manifest.primary));
    await expect(status).toHaveText("Live", { timeout: 20_000 });

    await daemon.stop();
    await expect(status).not.toHaveText("Live", { timeout: 15_000 });
    daemon = await startScenarioDaemon(manifest, { port });

    await expect(status).toHaveText("Live", { timeout: 45_000 });
    // The read that found the daemon back opened a stream the proxy holds: two missed beats (12 s), then the pace.
    const back = reads;
    await expect.poll(() => reads, { timeout: 30_000 }).toBeGreaterThanOrEqual(back + 2);
  } finally {
    await proxy.close();
    await daemon.stop();
    rmSync(root, { recursive: true, force: true });
  }
});
