// Loaded first in a child process (node --import tsx --import <this file> ...): the process records each sync of a
// database's log it makes to the file RERAIL_TEST_LOG_SYNCS names, for a test's power cut (`log-syncs.ts`).
import { recordLogSyncs } from "../engine/log-syncs.js";

recordLogSyncs();
