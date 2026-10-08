from __future__ import annotations

from typing import TYPE_CHECKING, cast

if TYPE_CHECKING:
    from threading import Thread
    from queue import Queue


def reads_the_module_first(thread: Thread):
    from threading import Thread

    local: Thread


def reads_at_run_time(thread: Thread):
    from threading import Thread

    cast(Thread, thread)


def only_a_local_binding():
    from queue import Queue as LocalQueue

    local: LocalQueue
