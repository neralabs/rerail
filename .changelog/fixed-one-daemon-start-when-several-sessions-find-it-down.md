Coding agent sessions and hooks that find the Rerail daemon down at the same moment now start it once between them. Before, each one launched its own `rerail daemon start` process.
