from proj.celery import app


@app.task(bind=True, default_retry_delay=7200)
def remind(self, user_id):
    try:
        return user_id
    except OSError as exc:
        raise self.retry(exc=exc, countdown=86_400)


def schedule(user_id):
    remind.apply_async((user_id,), countdown=86_400)
