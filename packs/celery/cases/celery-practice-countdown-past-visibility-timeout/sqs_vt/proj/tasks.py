from celery import shared_task


@shared_task
def remind(user_id):
    return user_id


def schedule(user_id):
    remind.apply_async((user_id,), countdown=45 * 60)
    remind.apply_async((user_id,), countdown=3 * 60 * 60)
    remind.apply_async((user_id,), countdown=5 * 60 * 60)  # caught
