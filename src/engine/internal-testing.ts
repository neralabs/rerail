/**
 * Rerail 0.1.0 ships code rules in Log or Steer mode, the dashboard, and the `/rerail` commands. Everything else a
 * person or a coding agent can reach is in internal testing: it stays in the code but is not part of the public
 * release. Each such feature is tagged where it is defined with an `Internal testing:` comment, and every description
 * shown for it (CLI usage, MCP tools, harness tools and commands) goes through {@link internalTesting}. README.md's
 * "Internal testing" section lists them.
 */
export const INTERNAL_TESTING_LABEL = "[Internal testing]";

/** A description of a feature in internal testing, as a person or a coding agent reads it. */
export function internalTesting(description: string): string {
  return `${INTERNAL_TESTING_LABEL} ${description}`;
}
