from celery import Task
from django.db import transaction


class AfterCommitTask(Task):
    def apply_async(self, *args, **kwargs):
        transaction.on_commit(lambda: super(AfterCommitTask, self).apply_async(*args, **kwargs))
