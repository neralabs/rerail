import {
  createHash,
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
  randomUUID,
  sign,
  verify as verifySignature,
  type webcrypto,
} from "node:crypto";
import { readFile, rm, stat, writeFile, mkdir } from "node:fs/promises";
import { heldOnWindows, nameTaken, replaceFile } from "../engine/file-replace.js";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { z } from "zod";
import type { DeviceSecretStore } from "./device-keystore.js";
import { deviceSecretStore } from "./device-keystore.js";
import { stableJsonText } from "../engine/stable-json.js";

const PROFILE_SCHEMA_VERSION = 1;
const ACTIVATION_REQUEST_VERSION = 1;

const publicJwkSchema = z.object({
  kty: z.literal("OKP"),
  crv: z.literal("Ed25519"),
  x: z.string().min(1),
  kid: z.string().min(1).optional(),
  alg: z.literal("EdDSA").optional(),
  use: z.string().optional(),
}).strict();

export const activationTrustRootsSchema = z.object({
  schemaVersion: z.literal(1),
  issuer: z.string().url(),
  audience: z.string().min(1),
  keys: z.array(publicJwkSchema.extend({ kid: z.string().min(1) }).strict()).min(1),
}).strict();

export const customerProfileSchema = z.object({
  schemaVersion: z.literal(PROFILE_SCHEMA_VERSION),
  issuer: z.string().url(),
  audience: z.string().min(1),
  profileId: z.string().min(1),
  sequence: z.number().int().nonnegative(),
  customerId: z.string().min(1),
  deploymentId: z.string().min(1),
  deviceId: z.string().min(1),
  deviceKeyThumbprint: z.string().regex(/^sha256:[A-Za-z0-9_-]{43}$/u),
  controlPlane: z.object({
    url: z.string().url(),
    audience: z.string().min(1),
  }).strict(),
  policyTrust: z.object({
    keyId: z.string().min(1),
    algorithm: z.literal("Ed25519"),
    publicKey: publicJwkSchema,
  }).strict(),
  entitlement: z.object({
    features: z.array(z.string().min(1)),
    automaticUpdates: z.boolean(),
    updateChannel: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u),
    offlineUntil: z.string().datetime({ offset: true }),
  }).strict(),
  notBefore: z.string().datetime({ offset: true }),
  expiresAt: z.string().datetime({ offset: true }),
}).strict();

export const flattenedJwsSchema = z.object({
  protected: z.string().regex(/^[A-Za-z0-9_-]+$/u),
  payload: z.string().regex(/^[A-Za-z0-9_-]+$/u),
  signature: z.string().regex(/^[A-Za-z0-9_-]+$/u),
}).strict();

const protectedHeaderSchema = z.object({
  alg: z.literal("EdDSA"),
  kid: z.string().min(1),
  typ: z.literal("rerail-profile+jws"),
}).strict();

const storedActivationSchema = z.object({
  schemaVersion: z.literal(1),
  trustRoots: activationTrustRootsSchema,
  profile: flattenedJwsSchema,
}).strict();

const storedDeviceSecretSchema = z.object({
  schemaVersion: z.literal(1),
  privateKeyPem: z.string().min(1),
  deviceCredential: z.unknown().optional(),
}).strict();

export type ActivationTrustRoots = z.infer<typeof activationTrustRootsSchema>;
export type CustomerProfile = z.infer<typeof customerProfileSchema>;
export type FlattenedJws = z.infer<typeof flattenedJwsSchema>;

export interface VerifiedCustomerProfile {
  profile: CustomerProfile;
  envelope: FlattenedJws;
  trustRoots: ActivationTrustRoots;
}

export interface DeviceIdentity {
  publicKey: webcrypto.JsonWebKey;
  thumbprint: string;
}

interface InternalDeviceIdentity extends DeviceIdentity {
  privateKeyPem: string;
  storedSecret: z.infer<typeof storedDeviceSecretSchema>;
}

export interface ActivationRequest {
  schemaVersion: 1;
  requestId: string;
  createdAt: string;
  profileName: string;
  deploymentId: string;
  devicePublicKey: webcrypto.JsonWebKey;
  deviceKeyThumbprint: string;
  platform: string;
  architecture: string;
  rerailVersion: string;
}

export interface SignedActivationRequest {
  request: ActivationRequest;
  signature: string;
}

export type ActivationFetch = (input: URL, init: RequestInit) => Promise<Response>;

export interface OnlineActivationOptions {
  profileName: string;
  code: string;
  activationUrl: string;
  trustRoots: ActivationTrustRoots;
  rerailVersion: string;
  fetch?: ActivationFetch;
  now?: Date;
}

export interface OfflineActivationRequestOptions {
  profileName: string;
  deploymentId: string;
  rerailVersion: string;
  now?: Date;
}

function assertProfileName(profileName: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u.test(profileName)) {
    throw new Error(`Invalid Rerail profile name: ${profileName}`);
  }
}

function decodeBase64UrlJson(value: string, label: string): unknown {
  const decoded = Buffer.from(value, "base64url");
  if (decoded.toString("base64url") !== value) throw new Error(`Invalid base64url in ${label}`);
  try {
    return JSON.parse(decoded.toString("utf8")) as unknown;
  } catch {
    throw new Error(`Invalid JSON in ${label}`);
  }
}

function publicKeyThumbprint(publicKey: webcrypto.JsonWebKey): string {
  if (publicKey.kty !== "OKP" || publicKey.crv !== "Ed25519" || typeof publicKey.x !== "string") {
    throw new Error("Rerail device key must be an Ed25519 public JWK");
  }
  const canonical = `{"crv":"Ed25519","kty":"OKP","x":${JSON.stringify(publicKey.x)}}`;
  return `sha256:${createHash("sha256").update(canonical).digest("base64url")}`;
}

function assertProfileTime(profile: CustomerProfile, now: Date, mode: "activation" | "runtime"): void {
  const timestamp = now.getTime();
  const notBefore = Date.parse(profile.notBefore);
  const expiresAt = Date.parse(profile.expiresAt);
  const offlineUntil = Date.parse(profile.entitlement.offlineUntil);
  if (timestamp < notBefore) throw new Error(`Rerail profile is not valid before ${profile.notBefore}`);
  const deadline = mode === "activation" ? expiresAt : offlineUntil;
  if (timestamp > deadline) {
    throw new Error(mode === "activation"
      ? `Rerail profile expired at ${profile.expiresAt}`
      : `Rerail profile offline entitlement expired at ${profile.entitlement.offlineUntil}`);
  }
}

export function verifyCustomerProfile(
  envelopeInput: unknown,
  trustInput: unknown,
  expectedDeviceThumbprint: string,
  options: { now?: Date; mode?: "activation" | "runtime" | "signature" } = {},
): VerifiedCustomerProfile {
  const envelope = flattenedJwsSchema.parse(envelopeInput);
  const trustRoots = activationTrustRootsSchema.parse(trustInput);
  const header = protectedHeaderSchema.parse(decodeBase64UrlJson(envelope.protected, "JWS protected header"));
  const trustedKey = trustRoots.keys.find(key => key.kid === header.kid);
  if (trustedKey === undefined) throw new Error(`Untrusted Rerail activation signing key: ${header.kid}`);

  const signedBytes = Buffer.from(`${envelope.protected}.${envelope.payload}`, "ascii");
  const signature = Buffer.from(envelope.signature, "base64url");
  if (signature.toString("base64url") !== envelope.signature) throw new Error("Invalid Rerail profile signature encoding");
  const key = createPublicKey({ key: trustedKey as webcrypto.JsonWebKey, format: "jwk" });
  if (!verifySignature(null, signedBytes, key, signature)) throw new Error("Invalid Rerail profile signature");

  const profile = customerProfileSchema.parse(decodeBase64UrlJson(envelope.payload, "JWS payload"));
  if (profile.issuer !== trustRoots.issuer) throw new Error(`Unexpected Rerail profile issuer: ${profile.issuer}`);
  if (profile.audience !== trustRoots.audience) throw new Error(`Unexpected Rerail profile audience: ${profile.audience}`);
  if (profile.deviceKeyThumbprint !== expectedDeviceThumbprint) {
    throw new Error("Rerail profile is bound to a different device key");
  }
  if (options.mode !== "signature") assertProfileTime(profile, options.now ?? new Date(), options.mode ?? "activation");
  return { profile, envelope, trustRoots };
}

/** In the Rerail home: `rerail activate`'s profiles, one folder each. */
export const PROFILES_DIR = "profiles";
/** In the Rerail home: the name of the active profile. */
export const ACTIVE_PROFILE_FILE = "active-profile";

/** In a profile's lock folder: the activation that holds it. */
const PROFILE_LOCK_OWNER_FILE = "owner.json";
/** How long an activation waits for another to let go of the profile's lock. */
const PROFILE_LOCK_WAIT_MS = 5_000;
const PROFILE_LOCK_POLL_MS = 10;
/**
 * A lock taken longer ago than this was left by a process that never let go, even if its pid now names another
 * process. An activation writes a few files and one device secret, which on macOS may wait on a Keychain prompt.
 */
const PROFILE_LOCK_STALE_MS = 10 * 60_000;
/**
 * A lock folder with no owner is being made: its activation writes the owner just after the folder. One with none
 * after this was left by an activation that died in between, or by an older Rerail, which wrote none.
 */
const PROFILE_LOCK_UNOWNED_MS = 10_000;

const profileLockOwnerSchema = z.object({ pid: z.number().int(), token: z.string().min(1), at: z.number() });
type ProfileLockOwner = z.infer<typeof profileLockOwnerSchema>;

export interface ProfileStoreOptions {
  /** Whose errors for a lock folder being deleted count as the lock being held; this process's by default. */
  readonly platform?: NodeJS.Platform;
  /** Waits between tries while another activation holds the profile's lock. */
  readonly sleep?: (ms: number) => Promise<void>;
}

async function readProfileLockOwner(lockPath: string): Promise<ProfileLockOwner | undefined> {
  try {
    return profileLockOwnerSchema.parse(JSON.parse(await readFile(join(lockPath, PROFILE_LOCK_OWNER_FILE), "utf8")));
  } catch {
    // None yet, half written, or on Windows deleted with its folder as the holder lets go.
    return undefined;
  }
}

function processRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    // EPERM: it runs, as another user.
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Whether the profile lock at `lockPath`, held by `owner` when last read, was left by an activation that is gone. */
async function profileLockAbandoned(lockPath: string, owner: ProfileLockOwner | undefined): Promise<boolean> {
  if (owner !== undefined) return !processRunning(owner.pid) || Date.now() - owner.at > PROFILE_LOCK_STALE_MS;
  // With no owner written into it, the folder's time is when it was made. No folder: let go since the try.
  const changedAt = await stat(lockPath).then(info => info.mtimeMs, () => undefined);
  return changedAt !== undefined && Date.now() - changedAt > PROFILE_LOCK_UNOWNED_MS;
}

/**
 * Removes the abandoned lock `owner` left, unless another activation took it over meanwhile; true when the next try may
 * take it. False on Windows while another process is deleting that folder already, so the caller waits for the delete.
 */
async function takeOverProfileLock(lockPath: string, owner: ProfileLockOwner | undefined, platform: NodeJS.Platform): Promise<boolean> {
  if ((await readProfileLockOwner(lockPath))?.token !== owner?.token) return true;
  try {
    await rm(lockPath, { recursive: true, force: true });
    return true;
  } catch (error) {
    if (heldOnWindows(error, platform)) return false;
    throw error;
  }
}

export class CustomerProfileStore {
  private readonly profilesRoot: string;
  private readonly locksRoot: string;
  private readonly activeProfilePath: string;
  private readonly platform: NodeJS.Platform;
  private readonly sleep: (ms: number) => Promise<void>;

  constructor(private readonly home: string, options: ProfileStoreOptions = {}) {
    this.profilesRoot = join(home, PROFILES_DIR);
    this.locksRoot = join(this.profilesRoot, ".locks");
    this.activeProfilePath = join(home, ACTIVE_PROFILE_FILE);
    this.platform = options.platform ?? process.platform;
    this.sleep = options.sleep ?? (async ms => { await sleep(ms); });
  }

  private profileDirectory(profileName: string): string {
    assertProfileName(profileName);
    return join(this.profilesRoot, profileName);
  }

  /**
   * Runs `operation` holding the profile's lock (`profiles/.locks/<name>.lock`), waiting while another activation holds
   * it. A lock whose activation is gone is taken over.
   */
  private async withProfileLock<T>(profileName: string, operation: () => Promise<T>): Promise<T> {
    assertProfileName(profileName);
    await mkdir(this.locksRoot, { recursive: true, mode: 0o700 });
    const lockPath = join(this.locksRoot, `${profileName}.lock`);
    const token = randomUUID();
    await this.takeProfileLock(lockPath, profileName, token);
    try {
      return await operation();
    } finally {
      // An activation held past PROFILE_LOCK_STALE_MS may find its lock taken over: then it is another's to remove.
      if ((await readProfileLockOwner(lockPath))?.token === token) await rm(lockPath, { recursive: true, force: true });
    }
  }

  /** Makes the profile's lock folder and writes this activation into it as the owner, waiting while another holds it. */
  private async takeProfileLock(lockPath: string, profileName: string, token: string): Promise<void> {
    const deadline = Date.now() + PROFILE_LOCK_WAIT_MS;
    while (!(await this.makeLockFolder(lockPath))) {
      const owner = await readProfileLockOwner(lockPath);
      if (await profileLockAbandoned(lockPath, owner) && await takeOverProfileLock(lockPath, owner, this.platform)) continue;
      if (Date.now() >= deadline) throw new Error(`Timed out waiting for Rerail profile lock: ${profileName}`);
      await this.sleep(PROFILE_LOCK_POLL_MS);
    }
    const owner: ProfileLockOwner = { pid: process.pid, token, at: Date.now() };
    try {
      await writeFile(join(lockPath, PROFILE_LOCK_OWNER_FILE), JSON.stringify(owner), { mode: 0o600 });
    } catch (error) {
      await rm(lockPath, { recursive: true, force: true });
      throw error;
    }
  }

  /** Makes the lock folder; false when another activation holds it, or on Windows is still deleting it as it lets go. */
  private async makeLockFolder(lockPath: string): Promise<boolean> {
    try {
      await mkdir(lockPath, { mode: 0o700 });
      return true;
    } catch (error) {
      if (nameTaken(error, this.platform)) return false;
      throw error;
    }
  }

  private async optionalFile(path: string): Promise<string | undefined> {
    try {
      return await readFile(path, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
  }

  private async restoreFile(path: string, value: string | undefined): Promise<void> {
    if (value === undefined) {
      await rm(path, { force: true });
      return;
    }
    await writeFile(path, value, { mode: 0o600 });
  }

  async selectedProfileName(): Promise<string | undefined> {
    try {
      const profileName = (await readFile(this.activeProfilePath, "utf8")).trim();
      assertProfileName(profileName);
      return profileName;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
  }

  async activate(
    profileName: string,
    envelope: FlattenedJws,
    trustRoots: ActivationTrustRoots,
    expectedDeviceThumbprint: string,
    now = new Date(),
    afterActivation?: () => Promise<void>,
  ): Promise<VerifiedCustomerProfile> {
    const verified = verifyCustomerProfile(envelope, trustRoots, expectedDeviceThumbprint, { now, mode: "activation" });
    return await this.withProfileLock(profileName, async () => {
      const directory = this.profileDirectory(profileName);
      const activationPath = join(directory, "activation.json");
      const previousPath = join(directory, "previous-activation.json");
      await mkdir(directory, { recursive: true, mode: 0o700 });

      const priorActivation = await this.optionalFile(activationPath);
      const priorPreviousActivation = await this.optionalFile(previousPath);
      const priorSelection = await this.optionalFile(this.activeProfilePath);
      try {
        if (priorActivation !== undefined) {
          const active = storedActivationSchema.parse(JSON.parse(priorActivation) as unknown);
          const prior = verifyCustomerProfile(active.profile, active.trustRoots, expectedDeviceThumbprint, { now, mode: "signature" });
          if (verified.profile.customerId !== prior.profile.customerId || verified.profile.deploymentId !== prior.profile.deploymentId) {
            throw new Error("A profile update cannot change customer or deployment identity");
          }
          if (verified.profile.sequence < prior.profile.sequence) {
            throw new Error(`Rerail profile sequence rollback rejected: ${verified.profile.sequence} < ${prior.profile.sequence}`);
          }
          await writeFile(previousPath, priorActivation, { mode: 0o600 });
        }

        const stored = { schemaVersion: 1 as const, trustRoots, profile: envelope };
        await replaceFile(activationPath, `${JSON.stringify(stored, null, 2)}\n`, { mode: 0o600 });
        await this.select(profileName);
        await afterActivation?.();
        return verified;
      } catch (error) {
        await this.restoreFile(activationPath, priorActivation);
        await this.restoreFile(previousPath, priorPreviousActivation);
        await this.restoreFile(this.activeProfilePath, priorSelection);
        throw error;
      }
    });
  }

  async load(profileName: string, expectedDeviceThumbprint: string, now = new Date()): Promise<VerifiedCustomerProfile> {
    const activationPath = join(this.profileDirectory(profileName), "activation.json");
    const stored = storedActivationSchema.parse(JSON.parse(await readFile(activationPath, "utf8")) as unknown);
    return verifyCustomerProfile(stored.profile, stored.trustRoots, expectedDeviceThumbprint, { now, mode: "runtime" });
  }

  async select(profileName: string): Promise<void> {
    this.profileDirectory(profileName);
    await mkdir(this.home, { recursive: true, mode: 0o700 });
    await replaceFile(this.activeProfilePath, `${profileName}\n`, { mode: 0o600 });
  }

  async remove(profileName: string): Promise<void> {
    await rm(this.profileDirectory(profileName), { recursive: true, force: true });
    if (await this.selectedProfileName() === profileName) await rm(this.activeProfilePath, { force: true });
  }
}

export class CustomerActivationManager {
  constructor(
    private readonly store: CustomerProfileStore,
    private readonly secrets: DeviceSecretStore = deviceSecretStore(),
  ) {}

  private async identity(profileName: string): Promise<InternalDeviceIdentity> {
    assertProfileName(profileName);
    const existing = await this.secrets.read(profileName);
    let storedSecret: z.infer<typeof storedDeviceSecretSchema>;
    if (existing === undefined) {
      const { privateKey } = generateKeyPairSync("ed25519");
      storedSecret = {
        schemaVersion: 1,
        privateKeyPem: privateKey.export({ format: "pem", type: "pkcs8" }).toString(),
      };
      await this.secrets.write(profileName, JSON.stringify(storedSecret));
    } else {
      storedSecret = storedDeviceSecretSchema.parse(JSON.parse(existing) as unknown);
    }
    const privateKey = createPrivateKey(storedSecret.privateKeyPem);
    const publicKey = createPublicKey(privateKey).export({ format: "jwk" });
    return {
      privateKeyPem: storedSecret.privateKeyPem,
      publicKey,
      thumbprint: publicKeyThumbprint(publicKey),
      storedSecret,
    };
  }

  async deviceIdentity(profileName: string): Promise<DeviceIdentity> {
    const identity = await this.identity(profileName);
    return { publicKey: identity.publicKey, thumbprint: identity.thumbprint };
  }

  async createOfflineRequest(options: OfflineActivationRequestOptions): Promise<SignedActivationRequest> {
    const identity = await this.identity(options.profileName);
    const request: ActivationRequest = {
      schemaVersion: ACTIVATION_REQUEST_VERSION,
      requestId: randomUUID(),
      createdAt: (options.now ?? new Date()).toISOString(),
      profileName: options.profileName,
      deploymentId: options.deploymentId,
      devicePublicKey: identity.publicKey,
      deviceKeyThumbprint: identity.thumbprint,
      platform: process.platform,
      architecture: process.arch,
      rerailVersion: options.rerailVersion,
    };
    const text = stableJsonText(request);
    if (text === undefined) throw new Error("Unable to serialize Rerail activation request");
    const signature = sign(null, Buffer.from(text), createPrivateKey(identity.privateKeyPem)).toString("base64url");
    return { request, signature };
  }

  async activateFile(
    profileName: string,
    envelopeInput: unknown,
    trustInput: unknown,
    now = new Date(),
  ): Promise<VerifiedCustomerProfile> {
    const identity = await this.identity(profileName);
    const envelope = flattenedJwsSchema.parse(envelopeInput);
    const trustRoots = activationTrustRootsSchema.parse(trustInput);
    return await this.store.activate(profileName, envelope, trustRoots, identity.thumbprint, now);
  }

  async activateOnline(options: OnlineActivationOptions): Promise<VerifiedCustomerProfile> {
    assertProfileName(options.profileName);
    const endpoint = new URL("/v1/activations/exchange", options.activationUrl);
    if (endpoint.protocol !== "https:" && endpoint.hostname !== "localhost" && endpoint.hostname !== "127.0.0.1") {
      throw new Error("Rerail activation requires HTTPS outside localhost");
    }
    const identity = await this.identity(options.profileName);
    const response = await (options.fetch ?? fetch)(endpoint, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        code: options.code,
        devicePublicKey: identity.publicKey,
        deviceKeyThumbprint: identity.thumbprint,
        device: {
          platform: process.platform,
          architecture: process.arch,
          rerailVersion: options.rerailVersion,
        },
      }),
    });
    if (!response.ok) throw new Error(`Rerail activation failed with HTTP ${response.status}`);
    const body = z.object({
      profile: flattenedJwsSchema,
      deviceCredential: z.unknown().optional(),
    }).strict().parse(await response.json());
    if (body.deviceCredential === undefined) throw new Error("Rerail activation response requires a device credential");
    const now = options.now ?? new Date();
    const verified = verifyCustomerProfile(body.profile, options.trustRoots, identity.thumbprint, { mode: "activation", now });
    return await this.store.activate(
      options.profileName,
      verified.envelope,
      verified.trustRoots,
      identity.thumbprint,
      now,
      async () => {
        const previousSecret = JSON.stringify(identity.storedSecret);
        try {
          await this.secrets.write(options.profileName, JSON.stringify({
            ...identity.storedSecret,
            deviceCredential: body.deviceCredential,
          }));
        } catch (error) {
          await this.secrets.write(options.profileName, previousSecret);
          throw error;
        }
      },
    );
  }

  async load(profileName: string, now = new Date()): Promise<VerifiedCustomerProfile> {
    const identity = await this.identity(profileName);
    return await this.store.load(profileName, identity.thumbprint, now);
  }

  async remove(profileName: string): Promise<void> {
    await this.secrets.remove(profileName);
    await this.store.remove(profileName);
  }
}
