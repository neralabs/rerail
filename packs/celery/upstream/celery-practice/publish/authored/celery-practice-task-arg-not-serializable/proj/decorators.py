from celery import shared_task as st
from huey import RedisHuey
from task_manager.django.decorators import task as packaged_task

from proj.celery import app

huey = RedisHuey("proj")


def task(*args, **kwargs):
    # A project wrapper around the app's decorator: what it decorates is a Celery task.
    return app.task(*args, **kwargs)


periodic = app.task


@task(bind=True)
def wrapped(self, tags):
    return tags


@periodic
def aliased(tags):
    return tags


@st
def renamed(tags):
    return tags


# A Celery wrapper package's decorator, imported beside celery.
@packaged_task(priority=3)
def packaged(tags):
    return tags


# Look-alike: huey's decorator, an attribute named `task` on another library's object.
@huey.task()
def hueyed(tags):
    return tags
