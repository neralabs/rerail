from celery import shared_task
from celery.result import AsyncResult

from proj.pools import POOL
from proj.tasks import price


def compute(i):
    return i * 2


@shared_task
def gather(ids):
    results = [price.delay(i) for i in ids]
    return [r.get() for r in results]  # caught


@shared_task
def gather_by_id(ids):
    pending = {i: price.delay(i) for i in ids}
    return {i: pending[i].get() for i in pending}  # caught


@shared_task
def gather_loop(ids):
    results = [price.apply_async((i,)) for i in ids]
    totals = []
    for r in results:
        totals.append(r.get(timeout=5))  # caught
    return totals


@shared_task
def poll(task_id):
    r = AsyncResult(task_id)
    if r.ready():
        return r.get()  # caught
    return None


# ---- near misses: nothing below is reported ----


@shared_task
def thread_pool(ids):
    # A thread pool's apply_async is not a Celery send: its .get() waits on a thread.
    return [POOL.apply_async(compute, (i,)).get() for i in ids]


@shared_task
def ids_only(ids):
    sent = [price.delay(i) for i in ids]
    return [r.id for r in sent]


@shared_task
def plain_rows(rows):
    return [row.get("id") for row in rows]
