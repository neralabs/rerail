from celery import Celery

app = Celery("shop")
app.autodiscover_tasks()
