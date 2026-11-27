from celery import Celery

# Redis, no visibility_timeout: the broker's default of one hour applies.
app = Celery("proj", broker="redis://localhost:6379/0")
app.conf.broker_transport_options = {"global_keyprefix": "proj:"}
