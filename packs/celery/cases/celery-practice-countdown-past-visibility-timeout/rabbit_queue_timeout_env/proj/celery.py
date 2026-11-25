import os

from celery import Celery
from kombu import Queue

# The queue's consumer timeout comes from configuration: the team chose a value, so the rule leaves the run alone.
app = Celery("proj", broker="pyamqp://guest@localhost//")
app.conf.task_queues = [
    Queue("celery", queue_arguments={"x-consumer-timeout": int(os.environ.get("CONSUMER_TIMEOUT_MS", "86400000"))}),
]
