"use strict";
// Loaded in place of the plugin package where upstream's tests load it by name from its sources, an ES module whose
// default export is the plugin (`require("eslint-plugin-react-hooks").default`): the installed CommonJS build, which
// eslint-driver.mjs leaves on globalThis, as that module.
module.exports = { ...globalThis.validationPlugin, default: globalThis.validationPlugin };
