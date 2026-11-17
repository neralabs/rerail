`RERAIL_AUTOSTART=false` now keeps coding agent hooks and `rerail setup` from starting the daemon, as `RERAIL_AUTOSTART=0` does. Before, they read only `0`.
