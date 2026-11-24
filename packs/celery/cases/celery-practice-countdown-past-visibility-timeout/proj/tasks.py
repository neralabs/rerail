import datetime
from datetime import timedelta, timezone

from celery import shared_task

from proj.celery import app

TWO_HOURS = 2 * 60 * 60
ONE_DAY = timedelta(days=1)
TEN_MINUTES = 600


@app.task
def ship(order_id):
    return order_id


# Sends delayed past the hour.
def schedule(order_id, delay):
    ship.apply_async(args=[order_id], countdown=7200)  # caught
    ship.apply_async(args=[order_id], countdown=60 * 60 * 2)  # caught
    ship.apply_async((order_id,), countdown=TWO_HOURS)  # caught
    ship.apply_async((order_id,), countdown=5400.0)  # caught
    ship.apply_async((order_id,), countdown=max(delay, 3 * 3600))  # caught
    ship.s(order_id).apply_async(countdown=ONE_DAY.total_seconds())  # caught
    app.send_task("proj.tasks.ship", args=[order_id], countdown=timedelta(hours=3).total_seconds())  # caught
    ship.apply_async((order_id,), eta=datetime.datetime.now(timezone.utc) + timedelta(hours=2))  # caught
    ship.apply_async((order_id,), eta=datetime.datetime.utcnow() + timedelta(1))  # caught
    now = datetime.datetime.now(timezone.utc)
    ship.apply_async((order_id,), eta=now + timedelta(minutes=90))  # caught
    ship.apply_async((order_id,), eta=timedelta(days=2, hours=1) + now)  # caught


# Left alone: an hour or less, values the rule cannot bound, and dates it cannot read.
def schedule_safe(order_id, delay, when):
    ship.apply_async(args=[order_id], countdown=3600)
    ship.apply_async(args=[order_id], countdown=60 * 60)
    ship.apply_async(args=[order_id], countdown=TEN_MINUTES)
    ship.apply_async(args=[order_id], countdown=delay)
    ship.apply_async(args=[order_id], countdown=min(delay, 600))
    ship.apply_async(args=[order_id], countdown=min(delay, 86_400) - 86_000)
    ship.apply_async((order_id,), eta=when)
    ship.apply_async((order_id,), eta=datetime.datetime(2030, 1, 1))
    ship.apply_async((order_id,), eta=datetime.datetime.now(timezone.utc) + timedelta(minutes=30))
    ship.apply_async((order_id,), eta=when + timedelta(hours=2))
    ship.delay(order_id, 7200)
    schedule_later(order_id, countdown=7200)


def schedule_later(order_id, countdown):
    return order_id, countdown


# Retries delayed past the hour.
@shared_task(bind=True, max_retries=5)
def sync_index(self, doc_id, delay):
    try:
        return doc_id
    except OSError as exc:
        raise self.retry(exc=exc, countdown=min(delay, 86_400))  # caught


@app.task(bind=True)
def push_webhook(task, hook_id):
    try:
        return hook_id
    except OSError as exc:
        raise task.retry(exc=exc, countdown=4 * 60 * 60)  # caught


@app.task(bind=True, default_retry_delay=4 * 3600)  # caught
def refresh_feed(self, feed_id):
    try:
        return feed_id
    except OSError as exc:
        raise self.retry(exc=exc)


@shared_task(autoretry_for=(OSError,), default_retry_delay=TWO_HOURS)  # caught
def fetch_avatar(user_id):
    return user_id


@shared_task(  # the cap is reached: 60 * 2 ** 9 seconds before the tenth retry
    autoretry_for=(OSError,),
    retry_backoff=60,
    retry_backoff_max=6 * 3600,  # caught
    max_retries=10,
)
def mirror_file(file_id):
    return file_id


@shared_task(autoretry_for=(OSError,), retry_backoff=True, retry_backoff_max=7200, max_retries=None)  # caught
def poll_status(job_id):
    return job_id


# Left alone: short retries, a growing countdown with no cap (a gap), a retry delay on a task that never retries,
# backoff caps that the retries never reach, and a cap without backoff.
@shared_task(bind=True, max_retries=3, default_retry_delay=900)
def short_retry(self, item_id):
    try:
        return item_id
    except OSError as exc:
        raise self.retry(exc=exc, countdown=30 * 2**self.request.retries)


@app.task(default_retry_delay=7200)
def never_retries(item_id):
    return item_id


@shared_task(autoretry_for=(OSError,), retry_backoff=True, retry_backoff_max=7200)
def few_retries(item_id):
    return item_id


@shared_task(autoretry_for=(OSError,), retry_backoff=30, retry_backoff_max=7200, retry_kwargs={"max_retries": 5})
def few_retries_kwargs(item_id):
    return item_id


@shared_task(autoretry_for=(OSError,), retry_backoff_max=7200, max_retries=50)
def no_backoff(item_id):
    return item_id


@shared_task(retry_backoff=True, retry_backoff_max=7200, max_retries=50)
def no_autoretry(item_id):
    return item_id


class Client:
    def retry(self, countdown):
        return countdown


def other_retry(client: Client):
    client.retry(countdown=7200)
