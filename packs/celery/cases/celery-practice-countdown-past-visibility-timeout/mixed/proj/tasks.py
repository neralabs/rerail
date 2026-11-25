from celery import shared_task


@shared_task
def remind(user_id):
    return user_id


def schedule(user_id):
    remind.apply_async((user_id,), countdown=45 * 60)
    remind.apply_async((user_id,), countdown=2 * 60 * 60)  # caught
