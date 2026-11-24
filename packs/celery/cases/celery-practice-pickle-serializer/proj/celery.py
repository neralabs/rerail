import os

from celery import Celery

PICKLE = "pickle"
SAFE = ["json"]

app = Celery("proj", task_serializer="pickle")  # caught
app.conf.result_serializer = "pickle"  # caught
app.conf.accept_content = ["json", "pickle"]  # caught
app.conf.update(
    event_serializer=PICKLE,  # caught
    result_accept_content=("application/x-python-serialize",),  # caught
)
app.conf.update({"task_serializer": "pickle"})  # caught
app.conf["accept_content"] = {"json", PICKLE}  # caught

# Left alone: JSON, msgpack, yaml, and a value read from the environment (a gap).
app.conf.result_serializer = "json"
app.conf.event_serializer = "msgpack"
app.conf.accept_content = SAFE
app.conf.update(result_accept_content=["json", "yaml"])
app.conf.task_serializer = os.environ.get("CELERY_TASK_SERIALIZER", "pickle")
app.config_from_object("proj.celeryconfig")
