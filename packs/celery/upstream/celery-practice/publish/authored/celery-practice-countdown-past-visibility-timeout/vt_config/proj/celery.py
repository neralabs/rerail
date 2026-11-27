import os

from celery import Celery

app = Celery("proj", broker="redis://localhost:6379/0")
app.conf.update(broker_transport_options={"visibility_timeout": int(os.environ.get("VISIBILITY_TIMEOUT", "3600"))})
