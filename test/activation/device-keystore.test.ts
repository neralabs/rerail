import { mkdir, mkdtemp, readdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { deviceSecretStore, FileDeviceSecretStore, SystemDeviceSecretStore } from "../../src/activation/device-keystore.js";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

describe("device secret store selection", () => {
  it("runs this suite against the file store", () => {
    expect(process.env.RERAIL_DEVICE_SECRET_STORE).toBe("file");
    expect(deviceSecretStore()).toBeInstanceOf(FileDeviceSecretStore);
  });

  it("refuses the system store inside a test run instead of reaching the keychain", () => {
    expect(() => deviceSecretStore({ VITEST: "true" })).toThrow("Tests must not use the system keychain");
    expect(() => deviceSecretStore({ VITEST: "true", RERAIL_DEVICE_SECRET_STORE: "system" })).toThrow("system keychain");
    expect(deviceSecretStore({})).toBeInstanceOf(SystemDeviceSecretStore);
    expect(() => deviceSecretStore({ RERAIL_DEVICE_SECRET_STORE: "keychain" })).toThrow("must be system or file");
  });

  it("keeps file secrets owner-only under the Rerail home and removes them", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-device-secrets-"));
    directories.push(home);
    const store = deviceSecretStore({ VITEST: "true", RERAIL_DEVICE_SECRET_STORE: "file", RERAIL_HOME: home });
    expect(await store.read("profile-a")).toBeUndefined();
    await store.write("profile-a", "secret");
    expect(await store.read("profile-a")).toBe("secret");
    expect((await stat(join(home, "device-secrets", "profile-a"))).mode & 0o777).toBe(0o600);
    await store.remove("profile-a");
    expect(await store.read("profile-a")).toBeUndefined();
    await expect(store.write("../escape", "x")).rejects.toThrow("Invalid Rerail profile name");
  });

  it("lets concurrent writes to one profile both land and leaves no staging files behind", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-device-secrets-"));
    directories.push(home);
    const store = new FileDeviceSecretStore(join(home, "device-secrets"));
    await Promise.all(["first", "second", "third"].map(value => store.write("profile-a", value)));
    expect(["first", "second", "third"]).toContain(await store.read("profile-a"));
    await mkdir(join(home, "device-secrets", "profile-b"));
    await expect(store.write("profile-b", "secret")).rejects.toThrow();
    expect((await readdir(join(home, "device-secrets"))).sort()).toEqual(["profile-a", "profile-b"]);
  });
});
