A coding agent's hook that finds no daemon running now starts one for the next event; before, only a session's start did. With `RERAIL_AUTOSTART=0` it starts none, and `rerail status` says so.
