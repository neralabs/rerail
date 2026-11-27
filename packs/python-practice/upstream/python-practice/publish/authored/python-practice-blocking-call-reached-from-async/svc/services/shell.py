import os
import subprocess

from asgiref.sync import sync_to_async


def git_rev():
    return subprocess.check_output(["git", "rev-parse", "HEAD"], text=True)


def flush_disks():
    os.system("sync")


@sync_to_async
def offloaded_rev():
    return subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, check=True)


class Git:
    def __init__(self):
        self._available = None

    def check(self):
        # Probed once per process and cached.
        if self._available is None:
            subprocess.run(["git", "--version"], check=True)
            self._available = True

    def check_uncached(self):
        if self._available is None:
            subprocess.run(["git", "--version"], check=True)
        return True

    async def status(self):
        self.check()
        return self.check_uncached()  # caught
