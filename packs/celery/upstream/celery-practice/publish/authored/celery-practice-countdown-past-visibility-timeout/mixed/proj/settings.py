import os

# RabbitMQ by default, Redis when configured: either may be deployed, so the threshold is the larger default (one
# hour) and the message names no broker.
if os.environ.get("USE_REDIS"):
    CELERY_BROKER_URL = os.environ.get("REDIS_URL", "redis://localhost:6379/0")
else:
    CELERY_BROKER_URL = os.environ.get("AMQP_URL", "amqp://guest@localhost//")
