import os

from celery.schedules import crontab


def _mail_entries():
    # Reached from parse_beat_schedule through the call graph.
    return [{"env_key": "MAIL_CRON", "task": "proj.tasks.fetch_mail", "options": {"expires": 60}}]  # caught


def parse_beat_schedule():
    # Entries listed with their options first; each cron comes from the environment.
    schedule = {}
    tasks = [
        {"env_key": "SHIP_CRON", "task": "proj.tasks.ship", "options": {"expires": 60}},
        {"env_key": "CLEAN_CRON", "task": "proj.tasks.clean", "options": {"expires": 60}},  # caught
    ]
    for task in tasks + _mail_entries():
        minute, hour, day_month, month, day_week = os.environ[task["env_key"]].split(" ")
        schedule[task["env_key"]] = {
            "task": task["task"],
            "schedule": crontab(minute, hour, day_week, day_month, month),
            "options": task["options"],
        }
    return schedule
