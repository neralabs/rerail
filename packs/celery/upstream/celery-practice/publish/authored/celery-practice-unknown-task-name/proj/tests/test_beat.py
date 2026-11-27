from proj.celery import app


def test_send_unknown():
    app.send_task("proj.tasks.gone")
