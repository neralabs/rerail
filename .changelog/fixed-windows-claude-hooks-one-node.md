On Windows, when the first `node` on PATH is not the Node that ran set-up, Claude Code's hooks now start the recorded Node directly, so each hook event runs one Node process instead of two.
