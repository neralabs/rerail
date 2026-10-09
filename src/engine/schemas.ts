/**
 * The rule-document schemas without the engine's Node runtime: the package's `@neralabs/rerail/schemas` entry
 * point, which browser clients (the playground page) parse rules with.
 */
export { conversationRuleSchema } from "./catalog/rule-schema.js";
export { codeRuleSchema } from "./code-rule/code-rule-schema.js";
