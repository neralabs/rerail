from proj.factory import build_app as create_app
from proj.factory import make_celery

cel = make_celery()
queue_app = create_app("queue")


@cel.task
def ping():
    return "pong"


@queue_app.task(bind=True)
def nightly(self):
    return self.request.id
