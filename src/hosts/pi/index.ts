/**
 * The Pi extension: the package's `@neralabs/rerail/pi` entry point, apart from the root so that only it loads `typebox`
 * (Pi supplies it to extensions; it is an optional peer dependency).
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export { createPiConversationRulesExtension, type PiConversationRulesOptions } from "./pi-extension.js";
