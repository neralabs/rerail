/**
 * The child process through which a process that must not load the engine reads when a process started, or ends a
 * daemon of a release before the engine's proof: the uninstall, which deletes the engine's folder once its stop is done
 * (`processStartedAtInChild`, `endProcessStartedByInChild`). It loads the engine, prints one line and exits, and its exit
 * lets go of the engine's file.
 */
import { loadNativeBinding } from "./native/native.js";
import { engineReaderAnswer } from "./process-liveness.js";

const answer = engineReaderAnswer(process.argv.slice(2), loadNativeBinding);
process.stdout.write(answer.stdout);
process.stderr.write(answer.stderr);
process.exitCode = answer.exitCode;
