/** Every linter config reader set-up runs, in the order their notes are given. */
import { banditReader } from "./python/bandit-config.js";
import { biomeReader } from "./javascript/biome-config.js";
import { clippyReader } from "./rust/clippy-config.js";
import { eslintReader } from "./javascript/eslint-config.js";
import type { LinterReader } from "./linter-config.js";
import { oxlintReader } from "./javascript/oxlint-config.js";
import { phpcsReader } from "./php/phpcs-config.js";
import { phpstanReader } from "./php/phpstan-config.js";
import { psalmReader } from "./php/psalm-config.js";
import { ruffReader } from "./python/ruff-config.js";
import { sqlfluffReader } from "./sql/sqlfluff-config.js";

export const LINTER_READERS: readonly LinterReader[] = [
  eslintReader, oxlintReader, biomeReader, ruffReader, banditReader, clippyReader, phpstanReader, psalmReader, phpcsReader, sqlfluffReader,
];
