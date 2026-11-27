from proj.celery import app


@app.task
def cleanup():
    pass
