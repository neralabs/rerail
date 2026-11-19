On Linux, `rerail daemon stop` says the daemon stopped only once every thread of it has exited, so the daemon no longer holds its files, sockets and locks after the stop answers.
