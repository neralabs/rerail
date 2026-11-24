import queue
from queue import Empty


def drain_until_sentinel(q, handle):
    while True:
        try:
            message = q.get(timeout=1)
            if message is None:
                break
            handle(message)
        except queue.Empty:
            continue


def wait_for_reply(replies, check_alive):
    while True:
        try:
            reply = replies.get(timeout=5)
            break
        except Empty:
            check_alive()
    return reply


def wait_for_reply_or_fail(replies, check_alive):
    while True:  # caught
        try:
            reply = replies.get(timeout=5)
            break
        except (Empty, OSError):
            check_alive()
    return reply


def register_worker(client, cache, worker_id):
    from redis.exceptions import LockError
    from redis.lock import Lock

    while True:
        try:
            with Lock(client, "lock:workers", timeout=3, blocking_timeout=3):
                workers = cache.get("workers") or []
                cache.set("workers", [*workers, worker_id])
                break
        except LockError:
            ...
