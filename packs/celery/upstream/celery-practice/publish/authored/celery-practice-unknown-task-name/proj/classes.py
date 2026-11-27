from celery import Task

from proj import REBUILD_TASK
from proj import contract as names
from proj.celery import app
from proj.contract import CLEANUP_TASK


# Registered under a module constant, imported.
class Cleanup(Task):
    name = CLEANUP_TASK

    def run(self):
        self.app.send_task("proj.tasks.gone_from_task")  # caught
        return 1


# Registered under a constant re-exported by the package.
class Rebuild(Task):
    name = REBUILD_TASK

    def run(self):
        return 1


# A subclass of the run's task class, named through a module alias.
class Weekly(Cleanup):
    name = names.WEEKLY_TASK


# A task class built on the app's own Task.
class Nightly(app.Task):
    name = "proj.nightly_report"

    def run(self):
        return 1
