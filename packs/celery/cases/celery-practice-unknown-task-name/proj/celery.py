from celery import Celery
from celery.schedules import crontab

from proj.contract import OPERATION_TASK

app = Celery("proj")
app.config_from_object("django.conf:settings", namespace="CELERY")

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
    # Commented out, so never scheduled:
    # "old": {
    #     "task": "proj.tasks.gone",
    #     "schedule": crontab(hour=7),
    # },
}

# Entries listed before their schedules are added (the cron comes from the environment).
BEAT_TASKS = [
    {"env_key": "SHIP_CRON", "task": "proj.tasks.ship", "options": {"expires": 60}},
    {"env_key": "CLEAN_CRON", "task": "proj.tasks.clean", "options": {"expires": 60}},  # caught
]

# A dict with a "task" key that is no beat entry.
AUDIT_EVENT = {"task": "proj.tasks.gone", "user": "admin"}
