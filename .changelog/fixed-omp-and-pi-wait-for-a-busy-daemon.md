In OMP and Pi, an event now waits at most 12 seconds for a busy daemon, as a hook does: it waited up to 14 seconds, and 24 with `RERAIL_AUTOSTART=0`, before the agent could go on.
