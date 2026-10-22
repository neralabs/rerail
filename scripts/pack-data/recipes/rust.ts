/**
 * The rust pack: Clippy 0.1.99's answers on its own UI tests (copied from rust-clippy at the pinned commit) and on the
 * pinned ripgrep and tokio corpora, recorded by packs/rust/upstream/clippy/generate.py with the toolchain
 * packs/rust/upstream/common.py pins.
 */
import { join } from "node:path";

import { derivationEnvironment, type DeriveRecipe } from "../context.js";

/** generate.py and common.py need only Python 3.9's standard library (common.check_toolchain refuses older). */
const PYTHON = "python3";

/**
 * The derivation's environment without what would change rustc's or cargo's input on the machine running it: flags
 * and wrappers (a `--cfg` turns code on), a target directory or build target (generate.py chooses both).
 */
function rustEnvironment(): NodeJS.ProcessEnv {
  const env = derivationEnvironment();
  for (const key of Object.keys(env)) {
    if (/^(?:RUSTFLAGS|RUSTDOCFLAGS|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_.*|CARGO_TARGET_.*)$/u.test(key)) delete env[key];
  }
  return env;
}

export const recipes: DeriveRecipe[] = [
  {
    pack: "rust",
    tool: "clippy",
    // common.py imports scripts/s3/pack_fixtures.py (generate.py and common.py are keyed as packs/rust/upstream/ files).
    inputs: ["scripts/s3/pack_fixtures.py"],
    runtimes: ["rust"],
    platforms: {
      only: ["linux"],
      reason: "corpus-tokio.expected.json is Clippy's answer on the build of tokio for the host, and CI derives on Linux, where `#[cfg(target_os = \"linux\")]` code compiles too: tokio/src/process/unix/pidfd_reaper.rs leaves `skipped`, and examples/prewarm-fd-table.rs:57's `#[allow(dead_code)]` adds an allow_attributes and an allow_attributes_without_reason finding. The rest is the same on macOS",
    },
    async derive({ toolDirectory, cacheRoot, capture, exec, checkout, corpus }) {
      const env = rustEnvironment();
      const generator = join(toolDirectory, "generate.py");

      // The pins generate.py refuses anything else than: the toolchain, rust-clippy's commit, the corpora.
      const pins = JSON.parse(await capture([PYTHON, "-c",
        "import json, sys; sys.path.insert(0, sys.argv[1]); import common; "
        + "print(json.dumps({'toolchain': common.TOOLCHAIN, 'origin': common.CLIPPY_ORIGIN, 'commit': common.CLIPPY_COMMIT, 'corpora': sorted(common.CORPUS_COMMITS)}))",
        join(toolDirectory, "..")], { env })) as { toolchain: string; origin: string; commit: string; corpora: string[] };

      await exec(["rustup", "toolchain", "install", pins.toolchain, "--profile", "minimal", "--component", "clippy", "--no-self-update"], { env });

      // generate.py copies tests/ui{,-toml}/** and builds clippy_test_deps (cargo --locked, from crates.io); nothing else.
      const clippy = await checkout(pins.origin.slice(0, pins.origin.lastIndexOf("@")), pins.commit, { sparse: ["/tests/", "/clippy_test_deps/"] });
      // Kept between runs: clippy_test_deps' target directory and the corpora's (cargo's fingerprints say what is stale).
      const scratch = join(cacheRoot, "rust-clippy");
      await exec([PYTHON, generator, "fixtures", clippy, join(scratch, "fixtures")], { env });
      // Each corpus at the pack's pin; common.run_corpus writes tokio's Cargo.lock (CORPUS_LOCK_PUBLISH_TIMES).
      const corpora: string[] = [];
      for (const name of pins.corpora) corpora.push(`${name}=${await corpus(name)}`);
      await exec([PYTHON, generator, "corpus", join(scratch, "corpora"), ...corpora], { env });
    },
  },
];
