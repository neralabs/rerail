/**
 * `/rerail-start` in OMP and Pi: where the repository is not set up, the set-up's questions asked through the
 * coding agent's own UI (`select` for each question, `input` for a folder or coding agent ids), following
 * each answer's `next.args` until set-up is done, stopped, or fails. Claude Code and Cursor relay the
 * agent lane's text instead.
 */
import type { SetupOption, SetupStep } from "./setup-step.js";

/** One `--json` set-up run: `start …` or `setup …`, answered by the launcher. */
export type SetupStepRunner = (args: readonly string[]) => Promise<SetupStep>;

export interface SetupPickerUi {
  notify(message: string, level: "info" | "error"): void;
  select(title: string, options: string[]): Promise<string | undefined>;
  input(title: string, placeholder?: string): Promise<string | undefined>;
}

/** What the person reads when they close a question or leave an input empty. */
export const SETUP_CANCELLED = "Set-up stopped. Nothing was turned on. Type /rerail-start again when you’re ready.";

/** The first run: `rerail start --json`, which sets the repo up where it is not set up and starts it where it is. */
export const FIRST_SETUP_ARGS: readonly string[] = ["start", "--json"];

const optionLabel = (option: SetupOption): string => (option.detail === null ? option.label : `${option.label} — ${option.detail}`);

/** The arguments that give `option`, asking for its value first when it needs one; undefined when the person cancels. */
async function answerArgs(ui: SetupPickerUi, option: SetupOption): Promise<readonly string[] | undefined> {
  if (option.input === null) return option.next.args;
  const value = (await ui.input(option.input.prompt, option.input.hint))?.trim();
  if (value === undefined || value === "") return undefined;
  return [...option.next.args, option.input.flag, value];
}

/**
 * Run set-up to its end through `ui`. Each question's text goes out as a notice, then the question as a
 * select. Resolves to the last step (`done`, `stopped` or `error`, already shown), or undefined when the
 * person cancelled, which changes nothing.
 */
export async function pickSetup(ui: SetupPickerUi, run: SetupStepRunner, first: readonly string[] = FIRST_SETUP_ARGS): Promise<SetupStep | undefined> {
  let step = await run(first);
  for (;;) {
    if (step.question === null) {
      ui.notify(step.screen, step.step === "error" ? "error" : "info");
      return step;
    }
    if (step.screen.trim() !== "") ui.notify(step.screen, "info");
    const labels = step.question.options.map(optionLabel);
    const picked = step.question.options[labels.indexOf(await ui.select(step.question.prompt, labels) ?? "")];
    const args = picked === undefined ? undefined : await answerArgs(ui, picked);
    if (args === undefined) {
      ui.notify(SETUP_CANCELLED, "info");
      return undefined;
    }
    step = await run(args);
  }
}
