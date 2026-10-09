import { ADAPTER_CAPABILITIES } from "../../src/engine/adapter-contract.js";
import { hostAdapterContract } from "../support/host-adapter-contract.js";
import { ReferenceHostAdapter } from "../support/reference-host-adapter.js";

for (const profile of Object.values(ADAPTER_CAPABILITIES)) {
  hostAdapterContract({
    name: `reference ${profile.host}`,
    create: () => new ReferenceHostAdapter(profile),
  });
}
