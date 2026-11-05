# SPDX-License-Identifier: Apache-2.0
"""Serialize original source APIs that rewrite shared content-addressed files."""
from contextlib import contextmanager
import fcntl


@contextmanager
def recording_lock(root):
    path = root / 'tmp/rerail-original-recording.lock'
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)
