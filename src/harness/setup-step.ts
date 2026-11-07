/**
 * `rerail setup --json` (and `rerail start --json`): one run of set-up as one object, for a coding agent
 * that asks the person through its own UI (OMP's and Pi's `ctx.ui.select`/`input`) instead of relaying
 * text. The agent lane's side effects, the agent lane's answers, and plain text for the person.
 */
import { z } from "zod";

/** The questions set-up asks; each is also the step a run that stops there reports. */
export type SetupQuestionId = "sessions" | "read-repo" | "confirm-packs" | "pick-packs" | "no-repo";

/** `done`: set-up finished (started, picked packs, skipped, or already on). `stopped`: set up here and stopped. */
export type SetupStepName = SetupQuestionId | "done" | "stopped" | "error";

export interface SetupOptionInput {
  /** Append `[flag, value]` to the option's `next.args`. */
  readonly flag: "--repo" | "--agents" | "--packs";
  /** What to ask the person for the value. */
  readonly prompt: string;
  /** An example, or the values it takes. */
  readonly hint: string;
}

export interface SetupOption {
  /** A key from `SESSIONS_ANSWERS`, `READ_REPO_ANSWERS`, `CONFIRM_PACKS_ANSWERS`, `PICK_PACKS_ANSWERS` or `NO_REPO_ANSWERS`, in the question's order. */
  readonly answer: string;
  readonly label: string;
  /** The hint shown beside the label; null when there is none. */
  readonly detail: string | null;
  /** The arguments to run the launcher with to give this answer, starting with `setup` and carrying `--json`. */
  readonly next: { readonly args: readonly string[] };
  /** The answer needs a value from the person first (a folder, coding agent ids); null when it does not. */
  readonly input: SetupOptionInput | null;
}

export interface SetupQuestion {
  readonly id: SetupQuestionId;
  /** The question itself, one line. */
  readonly prompt: string;
  readonly options: readonly SetupOption[];
}

export interface SetupStep {
  readonly step: SetupStepName;
  /** What to show the person, as plain text: everything the run said, without the question line. */
  readonly screen: string;
  /** The question the run stopped at; null at `done`, `stopped` and `error`. */
  readonly question: SetupQuestion | null;
}

const setupOptionSchema = z.object({
  answer: z.string(),
  label: z.string(),
  detail: z.string().nullable(),
  next: z.object({ args: z.array(z.string()) }),
  input: z.object({ flag: z.enum(["--repo", "--agents", "--packs"]), prompt: z.string(), hint: z.string() }).nullable(),
});

/** Validates what `rerail setup --json` / `rerail start --json` printed, for callers that run the CLI. */
export const setupStepSchema: z.ZodType<SetupStep> = z.object({
  step: z.enum(["sessions", "read-repo", "confirm-packs", "pick-packs", "no-repo", "done", "stopped", "error"]),
  screen: z.string(),
  question: z.object({
    id: z.enum(["sessions", "read-repo", "confirm-packs", "pick-packs", "no-repo"]),
    prompt: z.string(),
    options: z.array(setupOptionSchema),
  }).nullable(),
});
