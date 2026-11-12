import { basename } from "node:path";
import { build, type BuildOptions, type OutputFile } from "esbuild";

/**
 * The Node version a test plays when it runs a built entry point as an older Node would start it: the recorded-first
 * fallback starts `node` from PATH once the recorded one is gone, and the CLI, the hooks and the status line each say
 * then that Rerail needs Node 24.
 */
export const OLDER_NODE_VERSION = "22.20.0";

/** Node flags under which this Node reports itself as {@link OLDER_NODE_VERSION}: `process.versions.node` reads it. */
export function olderNodeArguments(): string[] {
  const version = `Object.defineProperty(process.versions, "node", { value: ${JSON.stringify(OLDER_NODE_VERSION)} });`;
  return ["--import", `data:text/javascript,${encodeURIComponent(version)}`];
}

async function outputFiles(options: BuildOptions): Promise<OutputFile[]> {
  return (await build({ ...options, write: false, logLevel: "silent" })).outputFiles;
}

/**
 * The first lines, at most five, of the bundle `options` build that esbuild writes otherwise when it builds the bundle
 * for {@link OLDER_NODE_VERSION}'s Node, as `<file>:<line>: <text>`; none when the two builds are the same file. Built
 * for that Node, esbuild rewrites what its tables say that Node lacks, `using` among it for Node 22, so a bundle the
 * same as that build reaches its own Node check there rather than stopping on a SyntaxError. The bundle is compared
 * rather than run on a Node that parses as Node 22 does: `--no-js-explicit-resource-management` makes Node 24 refuse
 * `using` as Node 22 does, but makes Node 26.4.0 on Windows crash on any ES module.
 */
export async function linesRewrittenForOlderNode(options: BuildOptions): Promise<string[]> {
  const target = `node${OLDER_NODE_VERSION.split(".")[0]}`;
  const [built, older] = await Promise.all([outputFiles(options), outputFiles({ ...options, target })]);
  return built.flatMap((file, index) => {
    const olderText = older[index]?.text ?? "";
    if (file.text === olderText) return [];
    // Rewriting adds helpers and re-indents the lines around what it rewrites, so the lines the older build lacks,
    // indents aside, are the rewritten ones.
    const olderLines = new Set(olderText.split("\n").map(line => line.trim()));
    const rewritten = file.text.split("\n")
      .flatMap((line, at) => (olderLines.has(line.trim()) ? [] : [`${basename(file.path)}:${at + 1}: ${line.trim()}`]));
    return rewritten.length === 0 ? [`${basename(file.path)}: differs from its build for that Node`] : rewritten;
  }).slice(0, 5);
}
