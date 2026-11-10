/**
 * How a folder removal waits out a file Windows holds for a moment, given to `rm` or `rmSync`: 10 more tries, each
 * 100 ms later than the one before, about 5.5 s in all. A folder a running process keeps open stays held, and the
 * removal then fails. Every removal that retries uses these: `removeScratch`, the updater's staging cleanup,
 * `rerail install --remove`, and the tests' `removeFolder`.
 *
 * A module that imports nothing, so the tests' setup file can load it without loading `file-replace.ts` and its
 * `node:fs` imports before a test file mocks them.
 */
export const FOLDER_REMOVAL_RETRIES = { maxRetries: 10, retryDelay: 100 } as const;
