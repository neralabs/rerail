from proj.schedule import parse_beat_schedule

CELERY_BEAT_SCHEDULE = parse_beat_schedule()
CELERY_BEAT_SCHEDULE["item"] = {"task": "proj.tasks.add_item", "options": {"expires": 5}}  # caught
CELERY_BEAT_SCHEDULE["ship-item"] = {"task": "proj.tasks.ship", "options": {"expires": 5}}
