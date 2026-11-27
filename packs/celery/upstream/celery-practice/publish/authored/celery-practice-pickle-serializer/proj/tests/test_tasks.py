from proj.celery import app
from proj.tasks import ping


def test_ping():
    app.conf.task_serializer = "pickle"
    ping.apply_async((), serializer="pickle")
