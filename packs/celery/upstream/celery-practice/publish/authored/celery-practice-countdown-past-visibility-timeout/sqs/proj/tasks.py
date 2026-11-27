from datetime import timedelta

from django.utils import timezone

from proj.celery import app


@app.task
def remind(user_id):
    return user_id


def schedule(user_id, when):
    remind.apply_async((user_id,), countdown=45 * 60)  # caught
    remind.apply_async((user_id,), eta=timezone.now() + timedelta(hours=1))  # caught
    remind.apply_async((user_id,), eta=timezone.localtime() + timedelta(hours=1))  # caught
    remind.apply_async((user_id,), eta=timezone.localtime(when) + timedelta(hours=1))
    remind.apply_async((user_id,), countdown=30 * 60)
    remind.apply_async((user_id,), countdown=600)
