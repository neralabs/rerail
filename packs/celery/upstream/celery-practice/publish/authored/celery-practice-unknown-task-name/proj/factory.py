from celery import Celery


def make_celery():
    return Celery("proj")


def build_app(name):
    created = Celery(name)
    created.conf.task_default_queue = name
    return created
