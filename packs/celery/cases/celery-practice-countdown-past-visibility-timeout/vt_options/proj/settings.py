import json
import os

CELERY_BROKER_URL = os.environ["BROKER_URL"]
CELERY_BROKER_TRANSPORT_OPTIONS = json.loads(os.environ.get("BROKER_TRANSPORT_OPTIONS", "{}"))
