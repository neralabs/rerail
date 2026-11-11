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
import { mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
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
  PROFILES_DIR,
  type ActivationFetch,
  CustomerProfileStore,
  type ActivationTrustRoots,
  type CustomerProfile,
  type FlattenedJws,
} from "../../src/activation/customer-profile.js";
import type { DeviceSecretStore } from "../../src/activation/device-keystore.js";
import { stableJsonText } from "../../src/engine/index.js";
import { holdRemovals } from "../support/platform/held-removal.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { raceForLock, raceSize, sourceModule } from "../support/processes/lock-race.js";

// Spied on so a test can make Windows hold a profile's folder for a moment.
vi.mock("node:fs/promises", { spy: true });
const { rm: actualRm } = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

const NOW = new Date("2026-09-01T12:00:00.000Z");
const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.mocked(rm).mockReset();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

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

async function fixture(): Promise<ProfileFixture> {
  const home = await mkdtemp(join(tmpdir(), "rerail-profile-"));
  temporaryDirectories.push(home);
  const profileStore = new CustomerProfileStore(home);
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

  it("removes a profile whose folder Windows holds for a moment", async () => {
    const setup = await fixture();
    const folder = join(setup.home, PROFILES_DIR, "acme");
    await mkdir(folder, { recursive: true });
    await writeFile(join(folder, "activation.json"), "{}");
    const refused = holdRemovals(vi.mocked(rm), actualRm, 2);

    await setup.profileStore.remove("acme");

    expect(refused).toEqual([folder, folder]);
    expect(existsSync(folder)).toBe(false);
  });

  it("rejects trailing profile command arguments", async () => {
    const setup = await fixture();
    await expect(runProfileCommand(["status", "unexpected"], setup.activation, setup.profileStore)).rejects
      .toThrow("Unknown profile argument: unexpected");
  });
});

describe("an activation finding its profile's lock taken", () => {
  it("waits for the activation that holds it, then takes the lock", async () => {
    const setup = await fixture();
    const thumbprint = (await setup.activation.deviceIdentity("acme")).thumbprint;
    const envelope = signProfile(profile(thumbprint), setup.signingKey);
    const events: string[] = [];
    const otherHolds = Promise.withResolvers<void>();
    const otherMayEnd = Promise.withResolvers<void>();
    const other = setup.profileStore.activate("acme", envelope, setup.trustRoots, thumbprint, NOW, async () => {
      events.push("the other holds the lock");
      otherHolds.resolve();
      await otherMayEnd.promise;
      events.push("the other lets go");
    });
    await otherHolds.promise;
    // This activation finds the lock held; its first wait lets the other end. No real time passes.
    const store = new CustomerProfileStore(setup.home, { sleep: async () => { otherMayEnd.resolve(); await other; } });

    await store.activate("acme", envelope, setup.trustRoots, thumbprint, NOW, async () => { events.push("this one holds the lock"); });
    expect(events).toEqual(["the other holds the lock", "the other lets go", "this one holds the lock"]);
  });
});

describe("activations racing for the lock of one killed while it held it", () => {
  it("take it one at a time", async () => {
    const setup = await fixture();
    const thumbprint = (await setup.activation.deviceIdentity("acme")).thumbprint;
    const signed = { envelope: signProfile(profile(thumbprint), setup.signingKey), trustRoots: setup.trustRoots, thumbprint, now: NOW };
    const size = raceSize({ processes: 8, rounds: 4 });

    const race = await raceForLock({
      take: `import { CustomerProfileStore } from ${JSON.stringify(sourceModule("src/activation/customer-profile.ts"))};
const { envelope, trustRoots, thumbprint, now } = JSON.parse(process.env.RERAIL_RACE_PROFILE);
const store = new CustomerProfileStore(${JSON.stringify(setup.home)});
const take = inside => store.activate("acme", envelope, trustRoots, thumbprint, new Date(now), inside);`,
      env: { RERAIL_RACE_PROFILE: JSON.stringify(signed) },
      ...size,
    });

    expect(race).toEqual({ overlaps: 0, ran: size.processes * size.rounds, busy: 0, errors: [] });
  }, 600_000);
});
