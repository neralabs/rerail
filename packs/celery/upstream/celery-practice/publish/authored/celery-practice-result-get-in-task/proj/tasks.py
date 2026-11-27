import celery.result
from celery import chain, chord, group, shared_task
from celery.result import AsyncResult, GroupResult, ResultSet, allow_join_result

from proj.celery import app
from proj.helpers import collect_totals, joined_rows, render_report


@shared_task
def price(item_id):
    return item_id * 2


@shared_task
def total(prices):
    return sum(prices)


@app.task(bind=True)
def checkout(self, item_ids):
    first = price.delay(item_ids[0]).get()  # caught
    pending = price.apply_async((item_ids[1],), countdown=1)
    second = pending.get(timeout=10)  # caught
    sent = app.send_task("proj.tasks.price", args=(3,))
    third = sent.wait()  # caught
    by_id = AsyncResult(self.request.id)
    by_id.get()  # caught
    self.AsyncResult(item_ids[2]).get()  # caught
    app.AsyncResult(item_ids[3]).get(propagate=False)  # caught
    celery.result.AsyncResult(item_ids[4]).get()  # caught
    GroupResult.restore(item_ids[5]).join()  # caught
    group(price.s(i) for i in item_ids).apply_async().get()  # caught
    fanned = group([price.s(i) for i in item_ids])()
    fanned.join_native()  # caught
    chord([price.s(i) for i in item_ids])(total.s()).get()  # caught
    chain(price.s(1), total.s())().get()  # caught
    (price.s(1) | total.s()).apply_async().get()  # caught
    sig = price.s(7)
    sig.delay().get()  # caught
    ResultSet([price.delay(i) for i in item_ids]).join()  # caught
    for value in price.delay(8).collect():  # caught
        print(value)
    return first + second + third


@shared_task
def report(account_id):
    return render_report(account_id)


@shared_task
def nightly(account_ids):
    return collect_totals(account_ids)


# ---- near misses: nothing below is reported ----


@shared_task
def opted_out(item_id):
    # Deliberate: the caller accepts the risk.
    return price.delay(item_id).get(disable_sync_subtasks=False)


@shared_task
def inside_allow_join(item_id):
    with allow_join_result():
        value = price.delay(item_id).get()
    return value


@shared_task
def status_only(item_id):
    result = price.delay(item_id)
    if result.ready() and result.successful():
        return result.state
    return result.id


@shared_task
def not_results(options, cache):
    timeout = options.get("timeout")
    cached = cache.get("price")
    worker = options.get("worker")
    return worker.join(timeout), cached


@shared_task
def chained_without_waiting(item_ids):
    # Chaining instead of waiting: the documented fix.
    chain(price.s(item_ids[0]), total.s()).apply_async()
    return group(price.s(i) for i in item_ids).apply_async().id


@shared_task
def rebinds(item_id):
    result = price.delay(item_id)
    result = {"id": result.id}
    return result.get("id")


def plain_helper(item_id):
    # A plain function no task reaches.
    return price.delay(item_id).get()


@shared_task
def helper_inside_allow_join(account_id):
    with allow_join_result():
        return joined_rows(account_id)
