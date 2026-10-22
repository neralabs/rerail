/** Internal testing: activation profiles, `rerail activate`, `rerail activation request`, `rerail profile` and `--profile`. */
import { readFile, writeFile } from "node:fs/promises";
import {
  CustomerActivationManager,
  CustomerProfileStore,
  activationTrustRootsSchema,
  flattenedJwsSchema,
  type VerifiedCustomerProfile,
} from "../activation/customer-profile.js";

export interface ParsedOption {
  args: string[];
  value?: string;
}

export interface SignedUpdatePolicy {
  localEngine: boolean;
  profileName: string;
  automaticUpdates: boolean;
  channel: string;
}

export function takeOption(args: readonly string[], name: string): ParsedOption {
  const remaining = [...args];
  const index = remaining.indexOf(name);
  if (index === -1) return { args: remaining };
  const value = remaining[index + 1];
  if (value === undefined || value.startsWith("--")) throw new Error(`${name} requires a value`);
  remaining.splice(index, 2);
  if (remaining.includes(name)) throw new Error(`${name} may be specified only once`);
  return { args: remaining, value };
}

function requiredOption(args: readonly string[], name: string): ParsedOption & { value: string } {
  const parsed = takeOption(args, name);
  if (parsed.value === undefined) throw new Error(`${name} is required`);
  return { args: parsed.args, value: parsed.value };
}

async function readJson(path: string): Promise<unknown> {
  try {
    return JSON.parse(await readFile(path, "utf8")) as unknown;
  } catch (error) {
    if (error instanceof SyntaxError) throw new Error(`Invalid JSON file: ${path}`);
    throw error;
  }
}

export async function selectedVerifiedProfile(
  manager: CustomerActivationManager,
  store: CustomerProfileStore,
  explicitProfileName?: string,
  now = new Date(),
): Promise<{ profileName: string; verified: VerifiedCustomerProfile } | undefined> {
  const profileName = explicitProfileName ?? await store.selectedProfileName();
  if (profileName === undefined) return undefined;
  return { profileName, verified: await manager.load(profileName, now) };
}

export async function signedUpdatePolicy(
  manager: CustomerActivationManager,
  store: CustomerProfileStore,
  explicitProfileName?: string,
  now = new Date(),
): Promise<SignedUpdatePolicy | undefined> {
  const selected = await selectedVerifiedProfile(manager, store, explicitProfileName, now);
  if (selected === undefined) return undefined;
  const features = selected.verified.profile.entitlement.features;
  return {
    profileName: selected.profileName,
    automaticUpdates: selected.verified.profile.entitlement.automaticUpdates
      && features.includes("automatic-updates"),
    channel: selected.verified.profile.entitlement.updateChannel,
    localEngine: features.includes("local-policy-kernel"),
  };
}

export function authorizedUpdateChannel(
  policy: SignedUpdatePolicy | undefined,
  requestedChannel: string | undefined,
): string | undefined {
  if (policy !== undefined && requestedChannel !== undefined && requestedChannel !== policy.channel) {
    throw new Error(`Signed profile ${policy.profileName} restricts updates to channel ${policy.channel}`);
  }
  return policy?.channel ?? requestedChannel;
}

export function assertLocalEngineEntitlement(policy: SignedUpdatePolicy | undefined): void {
  if (policy !== undefined && !policy.localEngine) {
    throw new Error(`Signed profile ${policy.profileName} does not entitle the local policy kernel`);
  }
}

export async function runActivateCommand(
  args: readonly string[],
  manager: CustomerActivationManager,
  rerailVersion: string,
  profileName = "default",
): Promise<number> {
  const trustOption = requiredOption(args, "--trust-root");
  const trustRoots = activationTrustRootsSchema.parse(await readJson(trustOption.value));
  const fileOption = takeOption(trustOption.args, "--file");
  const codeOption = takeOption(fileOption.args, "--code");
  const urlOption = takeOption(codeOption.args, "--activation-url");
  if (urlOption.args.length > 0) throw new Error(`Unknown activation argument: ${urlOption.args[0]}`);
  if ((fileOption.value === undefined) === (codeOption.value === undefined)) {
    throw new Error("Specify exactly one of --file or --code");
  }

  let verified: VerifiedCustomerProfile;
  if (fileOption.value === undefined) {
    const activationUrl = urlOption.value ?? process.env.RERAIL_ACTIVATION_URL;
    if (activationUrl === undefined) {
      throw new Error("--activation-url or RERAIL_ACTIVATION_URL is required for code activation");
    }
    if (codeOption.value === undefined) throw new Error("--code is required for code activation");
    verified = await manager.activateOnline({
      profileName,
      code: codeOption.value,
      activationUrl,
      trustRoots,
      rerailVersion,
    });
  } else {
    verified = await manager.activateFile(
      profileName,
      flattenedJwsSchema.parse(await readJson(fileOption.value)),
      trustRoots,
    );
  }
  process.stdout.write(`Activated profile ${profileName}\n`);
  process.stdout.write(`Customer: ${verified.profile.customerId}\n`);
  process.stdout.write(`Deployment: ${verified.profile.deploymentId}\n`);
  process.stdout.write(`Control plane: ${verified.profile.controlPlane.url}\n`);
  return 0;
}

export async function runActivationRequestCommand(
  args: readonly string[],
  manager: CustomerActivationManager,
  rerailVersion: string,
  explicitProfileName?: string,
): Promise<number> {
  const action = args[0];
  if (action !== "request") throw new Error(`Unknown activation action: ${action ?? "missing"}`);
  const deployment = requiredOption(args.slice(1), "--deployment");
  const output = requiredOption(deployment.args, "--output");
  if (output.args.length > 0) throw new Error(`Unknown activation request argument: ${output.args[0]}`);
  const request = await manager.createOfflineRequest({
    profileName: explicitProfileName ?? "default",
    deploymentId: deployment.value,
    rerailVersion,
  });
  await writeFile(output.value, `${JSON.stringify(request, null, 2)}\n`, { mode: 0o600 });
  process.stdout.write(`Wrote activation request to ${output.value}\n`);
  return 0;
}

export async function runProfileCommand(
  args: readonly string[],
  manager: CustomerActivationManager,
  store: CustomerProfileStore,
  explicitProfileName?: string,
): Promise<number> {
  if (args.length > 1) throw new Error(`Unknown profile argument: ${args[1]}`);
  const action = args[0] ?? "status";
  const profileName = explicitProfileName ?? await store.selectedProfileName();
  if (action === "status") {
    if (profileName === undefined) {
      process.stdout.write("No active Rerail customer profile.\n");
      return 0;
    }
    const verified = await manager.load(profileName);
    process.stdout.write(`Profile: ${profileName}\n`);
    process.stdout.write(`Customer: ${verified.profile.customerId}\n`);
    process.stdout.write(`Deployment: ${verified.profile.deploymentId}\n`);
    process.stdout.write(`Control plane: ${verified.profile.controlPlane.url}\n`);
    process.stdout.write(`Update channel: ${verified.profile.entitlement.updateChannel}\n`);
    process.stdout.write(`Offline until: ${verified.profile.entitlement.offlineUntil}\n`);
    return 0;
  }
  if (action === "select") {
    if (explicitProfileName === undefined) throw new Error("profile select requires --profile <name>");
    await manager.load(explicitProfileName);
    await store.select(explicitProfileName);
    process.stdout.write(`Selected profile ${explicitProfileName}\n`);
    return 0;
  }
  if (action === "remove") {
    if (profileName === undefined) throw new Error("No Rerail profile selected for removal");
    await manager.remove(profileName);
    process.stdout.write(`Removed profile ${profileName}\n`);
    return 0;
  }
  throw new Error(`Unknown profile action: ${action}`);
}
