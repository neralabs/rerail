from celery import Celery
from kombu import Queue

# The queue raises its own consumer timeout to three hours (RabbitMQ 3.12+, in milliseconds): the threshold.
THREE_HOURS_MS = 3 * 60 * 60 * 1000

app = Celery("proj", broker="amqp://guest@localhost//")
app.conf.task_queues = [Queue("celery", queue_arguments={"x-consumer-timeout": THREE_HOURS_MS})]
