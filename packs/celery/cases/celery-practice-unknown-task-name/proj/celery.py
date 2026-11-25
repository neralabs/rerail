from celery import Celery
from celery.schedules import crontab

from proj.contract import OPERATION_TASK

app = Celery("proj")
app.config_from_object("django.conf:settings", namespace="CELERY")

# Entries kept in module constants that the beat setting reads.
NIGHTLY_ENTRY = {"task": "proj.tasks.nightly_typo", "options": {"expires": 60}}  # caught
EXTRA_ENTRIES = {"extra": {"task": "proj.tasks.extra_typo", "options": {"expires": 60}}}  # caught

app.conf.beat_schedule = {
    "ship-every-hour": {
        "task": "proj.tasks.ship",
        "schedule": crontab(minute=0),
    },
    "ship-typo": {
        "task": "proj.tasks.shipp",  # caught
        "schedule": crontab(minute=5),
    },
    "moved-task": {
        "task": "proj.tasks.cleanup",  # caught
        "schedule": crontab(minute=10),
    },
    "tail-name": {
        "task": "tasks.ship",
        "schedule": crontab(minute=15),
    },
    "explicit-name": {
        "task": "reports.scheduler",
        "schedule": crontab(minute="*"),
    },
    "explicit-name-typo": {
        "task": "reports.schedular",  # caught
        "schedule": crontab(minute="*"),
    },
    "class-based": {
        "task": "exports.run",
        "schedule": crontab(hour=1),
    },
    "class-based-implicit": {
        "task": "proj.tasks.ArchiveTask",
        "schedule": crontab(hour=2),
    },
    "named-by-constant": {
        "task": "ops.execute",
        "schedule": crontab(hour=3),
    },
    "imported-constant": {
        "task": OPERATION_TASK,
        "schedule": crontab(hour=4),
    },
    # Celery's own task and a library's.
    "backend-cleanup": {
        "task": "celery.backend_cleanup",
        "schedule": crontab(hour=4, minute=30),
    },
    "beat-lib": {
        "task": "django_celery_beat.tasks.something",
        "schedule": crontab(hour=5),
    },
    # Another service's task: its module is not one of this run.
    "billing": {
        "task": "billing.tasks.charge",
        "schedule": crontab(hour=6),
    },
    # A factory-built app's task (`cel = make_celery()`, `@cel.task def ping` in proj/worker.py).
    "ping": {"task": "proj.worker.ping", "schedule": 60.0},
    # A Task subclass whose `name` is a module constant (proj/classes.py).
    "cleanup-class": {"task": "proj.cleanup", "schedule": crontab(minute=20)},
    # An entry given only its execution options, inside the beat setting.
    "options-only": {"task": "proj.tasks.shipx", "options": {"expires": 60}},  # caught
    "rebuild-class": {"task": "proj.rebuild_index", "schedule": crontab(minute=25)},
    "weekly-class": {"task": "proj.weekly_report", "schedule": crontab(minute=30)},
    "nightly-class": {"task": "proj.nightly_report", "schedule": crontab(minute=35)},
    "nightly-factory": {"task": "proj.worker.nightly", "schedule": crontab(minute=40)},
    "purge": {"task": "proj.jobs.purge", "schedule": crontab(minute=45)},
    "rotate": {"task": "ops.rotate", "schedule": crontab(minute=50)},
    "rotate-typo": {"task": "ops.rotat", "schedule": crontab(minute=55)},  # caught
    "nightly-entry": NIGHTLY_ENTRY,
    **EXTRA_ENTRIES,
    # Commented out, so never scheduled:
    # "old": {
    #     "task": "proj.tasks.gone",
    #     "schedule": crontab(hour=7),
    # },
}

# An item added to the beat setting, in dict() form.
app.conf.beat_schedule["other"] = dict(task="proj.tasks.missing", schedule=30.0)  # caught
app.conf.beat_schedule["ship-nightly"] = dict(task="proj.tasks.ship", schedule=crontab(hour=0))

app.conf.beat_schedule.update({"updated": {"task": "proj.tasks.update_typo", "options": {}}})  # caught

# A dict with a "task" key that is no beat entry.
AUDIT_EVENT = {"task": "proj.tasks.gone", "user": "admin"}
