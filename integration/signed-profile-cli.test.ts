import { describe, expect, test } from "vitest";
import { spawn } from "node:child_process";
import { generateKeyPairSync, randomUUID, sign, type KeyObject } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { text } from "node:stream/consumers";
import { join } from "node:path";
import type {
  ActivationTrustRoots,
  CustomerProfile,
  FlattenedJws,
  SignedActivationRequest,
} from "../src/customer-profile.js";

interface CommandResult {
  code: number;
  stdout: string;
  stderr: string;
}

async function runRerail(home: string, args: readonly string[]): Promise<CommandResult> {
  const child = spawn("node", ["dist/cli.js", ...args], {
    env: { ...process.env, RERAIL_AUTO_UPDATE: "0", RERAIL_HOME: home, RERAIL_DEVICE_SECRET_STORE: "file" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const [stdout, stderr, exitCode] = await Promise.all([
    text(child.stdout),
    text(child.stderr),
    new Promise<number>((resolve, reject) => {
      child.once("error", reject);
      child.once("close", code => resolve(code ?? 1));
    }),
  ]);
  return { code: exitCode, stdout, stderr };
}

function signedProfile(profile: CustomerProfile, privateKey: KeyObject): FlattenedJws {
  const protectedHeader = Buffer.from(JSON.stringify({
    alg: "EdDSA",
    kid: "activation-integration-1",
    typ: "rerail-profile+jws",
  })).toString("base64url");
  const payload = Buffer.from(JSON.stringify(profile)).toString("base64url");
  return {
    protected: protectedHeader,
    payload,
    signature: sign(null, Buffer.from(`${protectedHeader}.${payload}`, "ascii"), privateKey).toString("base64url"),
  };
}

describe("signed profile CLI", () => {
  test("requests, activates, enforces, reports, rejects invalid profiles, and removes a device-bound profile", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-signed-cli-"));
    const profileName = `integration-${randomUUID()}`;
    const requestPath = join(home, "activation-request.json");
    const profilePath = join(home, "customer-profile.jws");
    const tamperedProfilePath = join(home, "tampered-customer-profile.jws");
    const wrongRequestPath = join(home, "wrong-device-request.json");
    const rootsPath = join(home, "activation-trust-roots.json");
    const { privateKey, publicKey } = generateKeyPairSync("ed25519");
    const trustRoots: ActivationTrustRoots = {
      schemaVersion: 1,
      issuer: "https://activation.neralabs.test",
      audience: "rerail-customer-profile",
      keys: [{ ...publicKey.export({ format: "jwk" }), kid: "activation-integration-1" } as ActivationTrustRoots["keys"][number]],
    };
    await writeFile(rootsPath, JSON.stringify(trustRoots));

    try {
      const requested = await runRerail(home, [
        "--profile",
        profileName,
        "activation",
        "request",
        "--deployment",
        "acme-airgap",
        "--output",
        requestPath,
      ]);
      expect(requested).toMatchObject({ code: 0, stderr: "" });
      // The device key lives in the test's Rerail home, never in the system keychain.
      expect(existsSync(join(home, "device-secrets", profileName))).toBe(true);
      const request = JSON.parse(await readFile(requestPath, "utf8")) as SignedActivationRequest;
      const now = new Date();
      const profile: CustomerProfile = {
        schemaVersion: 1,
        issuer: trustRoots.issuer,
        audience: trustRoots.audience,
        profileId: "integration-profile",
        sequence: 1,
        customerId: "acme",
        deploymentId: "acme-airgap",
        deviceId: "integration-device",
        deviceKeyThumbprint: request.request.deviceKeyThumbprint,
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
          updateChannel: "latest",
          offlineUntil: new Date(now.getTime() + 24 * 60 * 60 * 1000).toISOString(),
        },
        notBefore: new Date(now.getTime() - 60_000).toISOString(),
        expiresAt: new Date(now.getTime() + 24 * 60 * 60 * 1000).toISOString(),
      };
      await writeFile(profilePath, JSON.stringify(signedProfile(profile, privateKey)));

      const activated = await runRerail(home, [
        "--profile",
        profileName,
        "activate",
        "--file",
        profilePath,
        "--trust-root",
        rootsPath,
      ]);
      expect(activated.code).toBe(0);
      expect(activated.stdout).toContain("Customer: acme");

      const status = await runRerail(home, ["--profile", profileName, "profile", "status"]);
      expect(status.code).toBe(0);
      expect(status.stdout).toContain("Deployment: acme-airgap");
      expect(status.stdout).toContain("Update channel: latest");

      const escapedChannel = await runRerail(home, ["--profile", profileName, "update", "check", "--channel", "beta"]);
      expect(escapedChannel.code).toBe(1);
      expect(escapedChannel.stderr).toContain("restricts updates to channel latest");

      const envelope = signedProfile(profile, privateKey);
      await writeFile(tamperedProfilePath, JSON.stringify({
        ...envelope,
        payload: Buffer.from(JSON.stringify({ ...profile, customerId: "attacker" })).toString("base64url"),
      }));
      const tampered = await runRerail(home, [
        "--profile", profileName, "activate", "--file", tamperedProfilePath, "--trust-root", rootsPath,
      ]);
      expect(tampered.code).toBe(1);
      expect(tampered.stderr).toContain("Invalid Rerail profile signature");

      const wrongProfileName = `${profileName}-wrong`;
      const wrongRequest = await runRerail(home, [
        "--profile", wrongProfileName, "activation", "request", "--deployment", "acme-airgap", "--output", wrongRequestPath,
      ]);
      expect(wrongRequest.code).toBe(0);
      const wrongDevice = await runRerail(home, [
        "--profile", wrongProfileName, "activate", "--file", profilePath, "--trust-root", rootsPath,
      ]);
      expect(wrongDevice.code).toBe(1);
      expect(wrongDevice.stderr).toContain("different device key");
      await runRerail(home, ["--profile", wrongProfileName, "profile", "remove"]);

      const removed = await runRerail(home, ["--profile", profileName, "profile", "remove"]);
      expect(removed.code).toBe(0);
      await expect(readFile(join(home, "active-profile"), "utf8")).rejects.toThrow();
      await expect(readFile(join(home, "profiles", profileName, "activation.json"), "utf8")).rejects.toThrow();
    } finally {
      await runRerail(home, ["--profile", profileName, "profile", "remove"]);
      await rm(home, { recursive: true, force: true });
    }
  }, 30_000);
});
