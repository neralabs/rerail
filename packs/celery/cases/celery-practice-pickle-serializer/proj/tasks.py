from celery import Task, shared_task

from proj.celery import app

PICKLE = "pickle"


@app.task(serializer="pickle")  # caught
def resize(image):
    return image


@shared_task(serializer=PICKLE)  # caught
def crop(image):
    return image


@shared_task(serializer="json")
def ping():
    return "pong"


class PickleTask(Task):
    serializer = "pickle"  # caught


class JsonTask(Task):
    serializer = "json"


def go(image):
    ping.apply_async((), serializer="pickle")  # caught
    ping.apply_async((), serializer="json")
    app.send_task("proj.tasks.ping", serializer="pickle")  # caught
    ping.delay()
