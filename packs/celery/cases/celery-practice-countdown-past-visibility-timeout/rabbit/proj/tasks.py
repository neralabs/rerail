from proj.celery import app


@app.task(bind=True, default_retry_delay=7200)
def remind(self, user_id):
    try:
        return user_id
    except OSError as exc:
        raise self.retry(exc=exc, countdown=86_400)  # caught


# RabbitMQ closes the channel of a consumer holding a message past consumer_timeout (30 minutes by default).
def schedule(user_id):
    remind.apply_async((user_id,), countdown=86_400)  # caught
    remind.apply_async((user_id,), countdown=2 * 60 * 60)  # caught
    remind.apply_async((user_id,), countdown=45 * 60)  # caught
    remind.apply_async((user_id,), countdown=30 * 60)
    remind.apply_async((user_id,), countdown=600)


@app.task(autoretry_for=(OSError,), retry_backoff=30, retry_backoff_max=2 * 60 * 60, max_retries=60)  # caught
def report_abuse(report_id):
    return report_id


# A decorator factory whose retry count is a local of the enclosing function.
def retryable_task(f):
    max_retries = 60

    @app.task(
        autoretry_for=(OSError,),
        retry_backoff=30,
        retry_backoff_max=2 * 60 * 60,  # caught
        retry_kwargs={"max_retries": max_retries},
        bind=True,
    )
    def wrapper(task, *args, **kw):
        return f(*args, **kw)

    return wrapper


def few_retries_task(f):
    max_retries = 3

    @app.task(autoretry_for=(OSError,), retry_backoff=30, retry_backoff_max=2 * 60 * 60, retry_kwargs={"max_retries": max_retries})
    def wrapper(*args, **kw):
        return f(*args, **kw)

    return wrapper
