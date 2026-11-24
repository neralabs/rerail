from celery import Celery

app = Celery("pickled")
app.conf.update(task_serializer="pickle", accept_content=["pickle"])
