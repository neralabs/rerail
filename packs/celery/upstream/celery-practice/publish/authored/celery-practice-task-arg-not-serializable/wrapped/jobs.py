from wrapped.celery import app
from wrapped.models import Event


@app.task
def publish(event):
    return event


def send(session, pk):
    event = session.get(Event, pk)
    publish.delay(event)
    publish.delay({pk})  # caught
