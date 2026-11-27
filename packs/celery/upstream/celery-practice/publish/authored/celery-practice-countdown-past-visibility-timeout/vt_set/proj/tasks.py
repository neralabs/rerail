from datetime import timedelta

from celery import shared_task
from django.utils import timezone


@shared_task
def remind(user_id):
    return user_id


def schedule(user_id):
    remind.apply_async((user_id,), countdown=7200)
    remind.apply_async((user_id,), countdown=12 * 60 * 60)
    remind.apply_async((user_id,), countdown=86_400)  # caught
    remind.apply_async((user_id,), eta=timezone.now() + timedelta(days=2))  # caught
