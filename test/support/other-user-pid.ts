/**
 * A live pid another OS user runs, or undefined when this process cannot name one: init's pid, unless it may signal it
 * (running as root, or as init's own user in a container). Windows has no such answer to `kill(pid, 0)`.
 */
export function otherUserPid(): number | undefined {
  if (process.platform === "win32") return undefined;
  try {
    process.kill(1, 0);
    return undefined;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM" ? 1 : undefined;
  }
}
