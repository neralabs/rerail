from celery import Task, shared_task

from proj.celery import app
from proj.contract import OPERATION_TASK


@shared_task
def ship(order_id):
    return order_id


@app.task(name="reports.scheduler")
def scheduler():
    pass


@shared_task(name=OPERATION_TASK)
def execute_operation(operation):
    return operation


def helper():
    pass


class ExportTask(Task):
    name = "exports.run"

    def run(self):
        pass


class ArchiveTask(Task):
    def run(self):
        pass


app.register_task(ExportTask())
app.register_task(ArchiveTask())
