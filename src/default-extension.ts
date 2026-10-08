import { createConversationRulesExtension } from "./extension.js";

// Hosts load the default; tests configure the factory from the same bundle so no unbundled imports are resolved by the host.
export { createConversationRulesExtension };
export default createConversationRulesExtension();
