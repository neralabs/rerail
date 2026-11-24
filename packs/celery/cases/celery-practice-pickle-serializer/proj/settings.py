# Django settings read with config_from_object("django.conf:settings", namespace="CELERY").
PICKLE = "pickle"
ACCEPTED = ["json", PICKLE]
CELERY_TASK_SERIALIZER = PICKLE  # caught
CELERY_ACCEPT_CONTENT = ACCEPTED  # caught
CELERY_RESULT_SERIALIZER = "json"
CELERY_EVENT_SERIALIZER = "msgpack"
CELERY_RESULT_ACCEPT_CONTENT = ("application/json",)

# Not Celery's settings: Django's session serializer and a cache option.
SESSION_SERIALIZER = "pickle"
CACHES = {"default": {"OPTIONS": {"SERIALIZER": "pickle"}}}


class CeleryConfig:
    CELERY_RESULT_SERIALIZER = "pickle"  # caught
    result_serializer = "pickle"  # caught
    accept_content = ("json", "application/x-python-serialize")  # caught
