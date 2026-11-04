`/api`, `/mcp` and `/native` refuse oversized bodies with `413` and slow requests with timeouts, and an internal error no longer shows its text to the caller; it goes to `daemon.log`.
