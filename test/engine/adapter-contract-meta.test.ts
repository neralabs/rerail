import { ADAPTER_CAPABILITIES } from "../src/adapter-contract.js";
import { hostAdapterContract } from "./contracts/host-adapter-contract.js";
import { ReferenceHostAdapter } from "./contracts/reference-host-adapter.js";

for (const profile of Object.values(ADAPTER_CAPABILITIES)) {
  hostAdapterContract({
    name: `reference ${profile.host}`,
    create: () => new ReferenceHostAdapter(profile),
  });
}
