"use strict";

/**
 * Responsible for loading / finding Mocha's "rc" files.
 *
 * @private
 * @module
 */

const fs = require("node:fs");
const path = require("node:path");
const debug = require("debug")("mocha:cli:config");
const { findUp } = require("find-up-simple");
const { createUnparsableFileError } = require("../errors.js");
const utils = require("../utils.cjs");

const findUpAny = async (files, options) => {
  for (const file of files) {
    const filepath = await findUp(file, options);
    if (filepath) {
      return filepath;
    }
  }

  return undefined;
};

/**
 * These are the valid config files, in order of precedence;
 * e.g., if `.mocharc.js` is present, then `.mocharc.yaml` and the rest
 * will be ignored.
 * The user should still be able to explicitly specify a file.
 * @private
 */
// This is documented publicly in `configuring.mdx`
exports.CONFIG_FILES = [
  ".mocharc.cjs",
  ".mocharc.js",
  ".mocharc.mjs",
  ".mocharc.yaml",
  ".mocharc.yml",
  ".mocharc.jsonc",
  ".mocharc.json",
];

/**
 * Parsers for various config filetypes. Each accepts a filepath and
 * returns an object (but could throw)
 */
const parsers = (exports.parsers = {
  yaml: async (filepath) =>
    require("js-yaml").load(await fs.promises.readFile(filepath, "utf8")),
  js: (filepath) => {
    let cwdFilepath;
    try {
      debug('parsers: load cwd-relative path: "%s"', path.resolve(filepath));
      cwdFilepath = require.resolve(path.resolve(filepath)); // evtl. throws
      return require(cwdFilepath);
    } catch (err) {
      if (cwdFilepath) throw err;

      debug('parsers: retry load as module-relative path: "%s"', filepath);
      return require(filepath);
    }
  },
  json: async (filepath) =>
    JSON.parse(
      require("strip-json-comments").default(
        await fs.promises.readFile(filepath, "utf8"),
      ),
    ),
});

/**
 * Loads and parses, based on file extension, a config file.
 * "JSON" files may have comments.
 *
 * @private
 * @param {string} filepath - Config file path to load
 * @returns {Promise<Object>} Parsed config object
 */
exports.loadConfig = async (filepath) => {
  let config;
  debug("loadConfig: trying to parse config at %s", filepath);

  const ext = path.extname(filepath);
  try {
    if (ext === ".yml" || ext === ".yaml") {
      config = await parsers.yaml(filepath);
    } else if (ext === ".js" || ext === ".cjs" || ext === ".mjs") {
      const parsedConfig = parsers.js(filepath);
      config = parsedConfig.default ?? parsedConfig;
    } else {
      config = await parsers.json(filepath);
    }
  } catch (err) {
    throw createUnparsableFileError(
      `Unable to read/parse ${filepath}: ${err}`,
      filepath,
    );
  }
  return config;
};

/**
 * Find ("find up") config file starting at `cwd`
 *
 * @param {string} [cwd] - Current working directory
 * @returns {Promise<string|undefined>} Filepath to config, if found
 */
exports.findConfig = async (cwd = utils.cwd()) => {
  const filepath = await findUpAny(exports.CONFIG_FILES, { cwd });
  if (filepath) {
    debug("findConfig: found config file %s", filepath);
  }
  return filepath;
};
