import os

from celery import Celery

# The broker comes from the environment: RabbitMQ when CLOUDAMQP_URL is set, else Redis. The scheme strings in
# replace() are not the broker's URL, so the rule does not take this run for RabbitMQ.
CLOUDAMQP_URL = os.getenv("CLOUDAMQP_URL", "")
BROKER_URL = CLOUDAMQP_URL.replace("amqps://", "pyamqp://", 1)
if not CLOUDAMQP_URL:
    BROKER_URL = os.environ["REDIS_URL"]

app = Celery("proj")
app.conf.update(broker_url=BROKER_URL)
