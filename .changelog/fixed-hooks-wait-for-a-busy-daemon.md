A hook no longer drops its event when the daemon is busy for a few seconds: it waits up to 10 seconds for the daemon to answer as itself, and sends nothing to a daemon that does not.
