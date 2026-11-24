import os

CELERY_BROKER_URL = os.environ.get("CELERY_BROKER_URL", "pyamqp://guest@localhost//")
