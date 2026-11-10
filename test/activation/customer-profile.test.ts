import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createPublicKey,
  generateKeyPairSync,
  sign,
  verify,
  type webcrypto,
  type KeyObject,
} from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  assertLocalEngineEntitlement,
  authorizedUpdateChannel,
  runProfileCommand,
  signedUpdatePolicy,
} from "../../src/cli/index.js";
import {
  ACTIVE_PROFILE_FILE,
  CustomerActivationManager,
  type ActivationFetch,
  CustomerProfileStore,
  type ActivationTrustRoots,
  type CustomerProfile,
  type FlattenedJws,
  type ProfileStoreOptions,
  type VerifiedCustomerProfile,
} from "../../src/activation/customer-profile.js";
import type { DeviceSecretStore } from "../../src/activation/device-keystore.js";
import { stableJsonText } from "../../src/engine/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";

vi.mock("node:fs/promises", { spy: true });
const { mkdir: realMkdir } = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

const NOW = new Date("2026-09-01T12:00:00.000Z");
const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.mocked(mkdir).mockReset();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** The next profile lock folder create fails with `code`, as Windows fails it while another activation deletes that folder. */
function failNextLockCreate(code: string): void {
  let failed = false;
  vi.mocked(mkdir).mockImplementation(async (...args: Parameters<typeof mkdir>) => {
    if (failed || !String(args[0]).endsWith(".lock")) return await realMkdir(...args);
    failed = true;
    throw Object.assign(new Error(`${code}: operation not permitted, mkdir '${String(args[0])}'`), { code, syscall: "mkdir" });
  });
}

class MemorySecretStore implements DeviceSecretStore {
  readonly values = new Map<string, string>();
  failNextWrite = false;

  async read(profileName: string): Promise<string | undefined> {
    return this.values.get(profileName);
  }

  async write(profileName: string, value: string): Promise<void> {
    if (this.failNextWrite) {
      this.failNextWrite = false;
      throw new Error("credential storage failed");
    }
    this.values.set(profileName, value);
  }

  async remove(profileName: string): Promise<void> {
    this.values.delete(profileName);
  }
}

interface ProfileFixture {
  home: string;
  activation: CustomerActivationManager;
  profileStore: CustomerProfileStore;
  secrets: MemorySecretStore;
  signingKey: KeyObject;
  trustRoots: ActivationTrustRoots;
}

async function fixture(options?: ProfileStoreOptions): Promise<ProfileFixture> {
  const home = await mkdtemp(join(tmpdir(), "rerail-profile-"));
  temporaryDirectories.push(home);
  const profileStore = new CustomerProfileStore(home, options);
  const secrets = new MemorySecretStore();
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const trustRoots: ActivationTrustRoots = {
    schemaVersion: 1,
    issuer: "https://activation.example.test",
    audience: "rerail-customer-profile",
    keys: [{ ...publicKey.export({ format: "jwk" }), kid: "activation-test-1" } as ActivationTrustRoots["keys"][number]],
  };
  return {
    home,
    activation: new CustomerActivationManager(profileStore, secrets),
    profileStore,
    secrets,
    signingKey: privateKey,
    trustRoots,
  };
}

function profile(deviceKeyThumbprint: string, sequence = 1, now = NOW): CustomerProfile {
  return {
    schemaVersion: 1,
    issuer: "https://activation.example.test",
    audience: "rerail-customer-profile",
    profileId: "profile-acme",
    sequence,
    customerId: "acme",
    deploymentId: "acme-us-prod",
    deviceId: "device-1",
    deviceKeyThumbprint,
    controlPlane: {
      url: "https://acme.rerail.test",
      audience: "rerail-acme",
    },
    policyTrust: {
      keyId: "acme-policy-1",
      algorithm: "Ed25519",
      publicKey: {
        kty: "OKP",
        crv: "Ed25519",
        x: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
      },
    },
    entitlement: {
      features: ["local-policy-kernel", "automatic-updates"],
      automaticUpdates: true,
      updateChannel: sequence === 1 ? "latest" : "beta",
      offlineUntil: new Date(now.getTime() + 30 * 24 * 60 * 60 * 1000).toISOString(),
    },
    notBefore: new Date(now.getTime() - 60_000).toISOString(),
    expiresAt: new Date(now.getTime() + 30 * 24 * 60 * 60 * 1000).toISOString(),
  };
}

function signProfile(value: CustomerProfile, signingKey: KeyObject): FlattenedJws {
  const protectedHeader = Buffer.from(JSON.stringify({
    alg: "EdDSA",
    kid: "activation-test-1",
    typ: "rerail-profile+jws",
  })).toString("base64url");
  const payload = Buffer.from(JSON.stringify(value)).toString("base64url");
  return {
    protected: protectedHeader,
    payload,
    signature: sign(null, Buffer.from(`${protectedHeader}.${payload}`, "ascii"), signingKey).toString("base64url"),
  };
}

describe("signed customer profiles", () => {
  it("activates a device-bound profile and makes its update entitlement authoritative", async () => {
    const setup = await fixture();
    const identity = await setup.activation.deviceIdentity("acme");
    const envelope = signProfile(profile(identity.thumbprint), setup.signingKey);

    const activated = await setup.activation.activateFile("acme", envelope, setup.trustRoots, NOW);
    expect(activated.profile).toMatchObject({ customerId: "acme", deploymentId: "acme-us-prod" });
    expect(await setup.profileStore.selectedProfileName()).toBe("acme");
    const policy = await signedUpdatePolicy(setup.activation, setup.profileStore, "acme", NOW);
    expect(authorizedUpdateChannel(policy, undefined)).toBe("latest");
    expect(() => authorizedUpdateChannel(policy, "beta")).toThrow("restricts updates to channel latest");
    expect(policy).toEqual({
      profileName: "acme",
      localEngine: true,
      automaticUpdates: true,
      channel: "latest",
    });
    expect(() => assertLocalEngineEntitlement({
      ...(policy as NonNullable<typeof policy>),
      localEngine: false,
    })).toThrow("does not entitle the local policy kernel");
  });

  it("rejects payload tampering, another device, expiration, and sequence rollback", async () => {
    const setup = await fixture();
    const identity = await setup.activation.deviceIdentity("acme");
    const first = signProfile(profile(identity.thumbprint), setup.signingKey);
    await setup.activation.activateFile("acme", first, setup.trustRoots, NOW);

    const tampered = { ...first, payload: Buffer.from(JSON.stringify({ ...profile(identity.thumbprint), customerId: "attacker" })).toString("base64url") };
    await expect(setup.activation.activateFile("acme", tampered, setup.trustRoots, NOW)).rejects.toThrow("Invalid Rerail profile signature");

    const otherIdentity = await setup.activation.deviceIdentity("other");
    await expect(setup.activation.activateFile("other", first, setup.trustRoots, NOW)).rejects.toThrow("different device key");

    await expect(setup.activation.activateFile(
      "expired",
      signProfile(profile((await setup.activation.deviceIdentity("expired")).thumbprint), setup.signingKey),
      setup.trustRoots,
      new Date(NOW.getTime() + 31 * 24 * 60 * 60 * 1000),
    )).rejects.toThrow("profile expired");

    await setup.activation.activateFile("acme", signProfile(profile(identity.thumbprint, 2), setup.signingKey), setup.trustRoots, NOW);
    await expect(setup.activation.activateFile("acme", first, setup.trustRoots, NOW)).rejects.toThrow("sequence rollback rejected");
  });

  it("signs offline activation requests with the generated device key", async () => {
    const setup = await fixture();
    const request = await setup.activation.createOfflineRequest({
      profileName: "acme",
      deploymentId: "acme-airgap",
      rerailVersion: "0.1.0",
      now: NOW,
    });
    const text = stableJsonText(request.request);
    expect(text).toBeDefined();
    expect(verify(
      null,
      Buffer.from(text as string),
      createPublicKey({ key: request.request.devicePublicKey as webcrypto.JsonWebKey, format: "jwk" }),
      Buffer.from(request.signature, "base64url"),
    )).toBe(true);
  });

  it("requires an online device credential and rolls back profile activation when credential storage fails", async () => {
    const setup = await fixture();
    const fakeFetch = (sequence: number, deviceCredential: unknown): ActivationFetch => async (_input, init) => {
      const request = JSON.parse(init?.body as string) as { deviceKeyThumbprint: string };
      return new Response(JSON.stringify({
        profile: signProfile(profile(request.deviceKeyThumbprint, sequence), setup.signingKey),
        deviceCredential,
      }), { status: 200, headers: { "content-type": "application/json" } });
    };

    await setup.activation.activateOnline({
      profileName: "acme",
      code: "one-use-code",
      activationUrl: "http://localhost:3000",
      trustRoots: setup.trustRoots,
      rerailVersion: "0.1.0",
      fetch: fakeFetch(1, { certificate: "old-device-certificate" }),
      now: NOW,
    });
    const priorSecret = setup.secrets.values.get("acme");
    setup.secrets.failNextWrite = true;
    await expect(setup.activation.activateOnline({
      profileName: "acme",
      code: "replacement-code",
      activationUrl: "http://localhost:3000",
      trustRoots: setup.trustRoots,
      rerailVersion: "0.1.0",
      fetch: fakeFetch(2, { certificate: "new-device-certificate" }),
      now: NOW,
    })).rejects.toThrow("credential storage failed");
    expect((await setup.activation.load("acme", NOW)).profile.sequence).toBe(1);
    expect(setup.secrets.values.get("acme")).toBe(priorSecret);

    await expect(setup.activation.activateOnline({
      profileName: "missing-credential",
      code: "one-use-code",
      activationUrl: "http://localhost:3000",
      trustRoots: setup.trustRoots,
      rerailVersion: "0.1.0",
      fetch: fakeFetch(1, undefined),
      now: NOW,
    })).rejects.toThrow("requires a device credential");
  });

  it("serializes concurrent profile replacements so a downgrade cannot win", async () => {
    const setup = await fixture();
    const identity = await setup.activation.deviceIdentity("acme");
    await setup.activation.activateFile("acme", signProfile(profile(identity.thumbprint), setup.signingKey), setup.trustRoots, NOW);
    await Promise.allSettled([
      setup.activation.activateFile("acme", signProfile(profile(identity.thumbprint, 3), setup.signingKey), setup.trustRoots, NOW),
      setup.activation.activateFile("acme", signProfile(profile(identity.thumbprint, 2), setup.signingKey), setup.trustRoots, NOW),
    ]);
    expect((await setup.activation.load("acme", NOW)).profile.sequence).toBe(3);
  });

  it("leaves no file beside the selection when it cannot be written", async () => {
    const setup = await fixture();
    // A folder where the selection goes: no platform renames a file over one.
    await mkdir(join(setup.home, ACTIVE_PROFILE_FILE, "kept"), { recursive: true });
    await expect(setup.profileStore.select("acme")).rejects.toThrow();
    expect(await readdir(setup.home)).toEqual([ACTIVE_PROFILE_FILE]);
  });

  it.each(["EPERM", "EACCES", "EBUSY"])("waits through %s on Windows, where another activation is deleting the profile lock folder", async code => {
    const setup = await fixture({ platform: "win32" });
    const identity = await setup.activation.deviceIdentity("acme");
    failNextLockCreate(code);

    const activated = await setup.activation.activateFile("acme", signProfile(profile(identity.thumbprint), setup.signingKey), setup.trustRoots, NOW);
    expect(activated.profile.customerId).toBe("acme");
  });

  it("fails at once on EPERM elsewhere, where it means the home refuses this process", async () => {
    const setup = await fixture({ platform: "linux" });
    const identity = await setup.activation.deviceIdentity("acme");
    failNextLockCreate("EPERM");

    await expect(setup.activation.activateFile("acme", signProfile(profile(identity.thumbprint), setup.signingKey), setup.trustRoots, NOW))
      .rejects.toMatchObject({ code: "EPERM" });
  });

  it("rejects trailing profile command arguments", async () => {
    const setup = await fixture();
    await expect(runProfileCommand(["status", "unexpected"], setup.activation, setup.profileStore)).rejects
      .toThrow("Unknown profile argument: unexpected");
  });
});

describe("an activation finding its profile's lock taken", () => {
  const neverWaits = { sleep: async (): Promise<void> => { throw new Error("waited"); } };

  it("takes over the lock of an activation whose process is gone", async () => {
    const setup = await fixture(neverWaits);
    const lockPath = await profileLock(setup.home, { owner: { pid: DEAD_PID, token: "other", at: Date.now() } });

    await expect(activateAcme(setup)).resolves.toMatchObject({ profile: { customerId: "acme" } });
    expect(existsSync(lockPath)).toBe(false);
  });

  it("takes over a lock older than any activation takes, even if its pid now names a running process", async () => {
    const setup = await fixture(neverWaits);
    await profileLock(setup.home, { owner: { pid: process.pid, token: "other", at: Date.now() - 11 * 60_000 } });

    await expect(activateAcme(setup)).resolves.toMatchObject({ profile: { customerId: "acme" } });
  });

  it("takes over a lock with no owner, left by an activation that died before writing one", async () => {
    const setup = await fixture(neverWaits);
    await profileLock(setup.home, { ageMs: 60_000 });

    await expect(activateAcme(setup)).resolves.toMatchObject({ profile: { customerId: "acme" } });
  });

  it.each([
    ["a running activation", { owner: { pid: process.pid, token: "other", at: Date.now() } }],
    ["an activation that has made the lock and not written its owner yet", {}],
  ])("waits for %s, then takes the lock", async (_holder, held) => {
    const events: string[] = [];
    let lockPath = "";
    const setup = await fixture({
      // While this activation waits, the other finishes and lets go.
      sleep: async () => {
        events.push(existsSync(lockPath) ? "the other holds the lock" : "the other lost its lock");
        await rm(lockPath, { recursive: true });
        events.push("the other lets go");
      },
    });
    lockPath = await profileLock(setup.home, held);

    await expect(activateAcme(setup)).resolves.toMatchObject({ profile: { customerId: "acme" } });
    expect(events).toEqual(["the other holds the lock", "the other lets go"]);
  });

  it("leaves the lock alone when another activation took it over meanwhile", async () => {
    const setup = await fixture();
    const identity = await setup.activation.deviceIdentity("acme");
    const lockPath = join(setup.home, "profiles", ".locks", "acme.lock");
    const theirs = JSON.stringify({ pid: process.pid, token: "theirs", at: Date.now() });
    // A secret store write held longer than PROFILE_LOCK_STALE_MS, after which another activation took the lock.
    setup.secrets.write = async () => { await writeFile(join(lockPath, "owner.json"), theirs); };

    await setup.activation.activateOnline({
      profileName: "acme",
      code: "one-use-code",
      activationUrl: "http://localhost:3000",
      trustRoots: setup.trustRoots,
      rerailVersion: "0.1.0",
      fetch: async () => new Response(JSON.stringify({ profile: signProfile(profile(identity.thumbprint), setup.signingKey), deviceCredential: {} })),
      now: NOW,
    });
    await expect(readFile(join(lockPath, "owner.json"), "utf8")).resolves.toBe(theirs);
  });
});

/** A pid no process has: above Linux's default limit, and odd, which no Windows pid is. */
const DEAD_PID = 2 ** 22 - 1;

/** `home`'s lock on the profile "acme" as another activation holds it: with `owner` written, made `ageMs` ago. */
async function profileLock(home: string, held: { owner?: { pid: number; token: string; at: number }; ageMs?: number }): Promise<string> {
  const lockPath = join(home, "profiles", ".locks", "acme.lock");
  await mkdir(lockPath, { recursive: true });
  if (held.owner !== undefined) await writeFile(join(lockPath, "owner.json"), JSON.stringify(held.owner));
  const madeAt = new Date(Date.now() - (held.ageMs ?? 0));
  await utimes(lockPath, madeAt, madeAt);
  return lockPath;
}

async function activateAcme(setup: ProfileFixture): Promise<VerifiedCustomerProfile> {
  const identity = await setup.activation.deviceIdentity("acme");
  return await setup.activation.activateFile("acme", signProfile(profile(identity.thumbprint), setup.signingKey), setup.trustRoots, NOW);
}
