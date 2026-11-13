/** Every linter config reader set-up runs, in the order their notes are given. */
import { banditReader } from "./bandit-config.js";
import { biomeReader } from "./biome-config.js";
import { clippyReader } from "./clippy-config.js";
import { eslintReader } from "./eslint-config.js";
import type { LinterReader } from "./linter-config.js";
import { oxlintReader } from "./oxlint-config.js";
import { phpcsReader } from "./phpcs-config.js";
import { phpstanReader } from "./phpstan-config.js";
import { psalmReader } from "./psalm-config.js";
import { ruffReader } from "./ruff-config.js";
import { sqlfluffReader } from "./sqlfluff-config.js";

export const LINTER_READERS: readonly LinterReader[] = [
  eslintReader, oxlintReader, biomeReader, ruffReader, banditReader, clippyReader, phpstanReader, psalmReader, phpcsReader, sqlfluffReader,
];
