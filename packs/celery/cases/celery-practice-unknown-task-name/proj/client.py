from celery import Celery, current_app

from proj.factory import make_celery

remote = Celery("remote", broker="redis://other")


class Mailer:
    def send_task(self, name):
        return name


mailer = Mailer()


def kick(upload_id):
    # Another service's task: this run has proj/tasks.py, but no top-level module `tasks`.
    remote.send_task("tasks.process_upload", args=[upload_id])
    # Its module part only ends like one of the run's (proj.web.views).
    remote.send_task("web.views.render", args=[upload_id])
    # A look-alike send_task on an object that is no Celery app.
    mailer.send_task("proj.tasks.gone")
    current_app.send_task("proj.tasks.gone_current")  # caught
    make_celery().send_task("proj.tasks.gone_factory")  # caught


# Jobs of another scheduler, with "task" and "options" keys but outside the beat setting.
ML_JOBS = [
    {"task": "metrics.weekly", "options": {"window": 7}},
    {"task": "proj.metrics.weekly", "options": {"window": 7}},
]


def export_jobs():
    return [{"task": "proj.metrics.monthly", "options": {"window": 30}}]
