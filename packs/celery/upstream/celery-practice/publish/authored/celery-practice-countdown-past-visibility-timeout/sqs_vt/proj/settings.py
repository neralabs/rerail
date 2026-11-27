import os

# SQS with its visibility timeout raised to four hours: the threshold.
TRANSPORT_OPTIONS = {"region": "eu-west-1", "visibility_timeout": 4 * 60 * 60}

CELERY_BROKER_URL = os.environ.get("CELERY_BROKER_URL", "sqs://")
CELERY_BROKER_TRANSPORT_OPTIONS = TRANSPORT_OPTIONS
