import type { RrRunRequest } from "../native/native.js";

/** What the runner asks its child process (`rr-child.ts`), in the request file. */
export type RrChildRequest = { kind: "describe"; paths: string[] } | { kind: "run"; run: RrRunRequest };

/** What the child answers, in the result file: the addon's answer, parsed, or the error it threw; and its peak resident memory. */
export type RrChildResult = ({ ok: true; value: unknown } | { ok: false; message: string }) & { peakRssBytes: number };

/** What the child sends its parent while it runs: its resident memory now. */
export interface RrMemoryReport {
  rssBytes: number;
}
