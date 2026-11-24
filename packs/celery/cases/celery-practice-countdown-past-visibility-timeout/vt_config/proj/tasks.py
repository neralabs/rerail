from proj.celery import app


@app.task
def remind(user_id):
    return user_id


def schedule(user_id):
    remind.apply_async((user_id,), countdown=86_400)
