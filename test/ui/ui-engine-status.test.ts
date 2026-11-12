import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { headerStatus } from "../../ui/frame/topbar.js";
import { data, setScope } from "../../ui/state.js";

const NO_STORE = "Rerail at /opt/rerail is neither the managed installation under /home/me/.rerail/versions nor in a Git checkout, so it has no content store";
const MODEL_OK = { ok: true, message: "ok" };

describe("the dashboard header and the daemon's engine", () => {
  beforeEach(() => {
    // A page whose polls come in for a repository that runs: the header's words come from the daemon's `/healthz`.
    data.loaded = true;
    data.updatedAt = Date.now();
    setScope({ repository: "repository-key" });
  });

  afterEach(() => {
    data.loaded = false;
    data.updatedAt = 0;
    data.health = null;
    setScope(null);
  });

  it("turns red with the reason when the engine is unavailable, since no code rule runs", () => {
    data.health = { version: "1.0.0", model: MODEL_OK, native: { ok: false, error: NO_STORE } };

    const status = headerStatus();

    expect(status).toMatchObject({ cls: "down", text: "Engine unavailable" });
    expect(status.title).toContain(NO_STORE);
  });

  it("stays live while the engine is ok", () => {
    data.health = { version: "1.0.0", model: MODEL_OK, native: { ok: true, kernelVersion: "1" } };

    expect(headerStatus()).toMatchObject({ cls: "", text: "Live" });
  });
});
