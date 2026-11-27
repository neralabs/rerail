from celery import shared_task

from proj.celery import app


def purge():
    pass


# Tasks made by calling the decorator as a function.
purge_task = app.task(purge)


def rotate():
    pass


rotate_task = shared_task(name="ops.rotate")(rotate)
