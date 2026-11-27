from celery import shared_task

from proj.celery import app


@app.task
def ship(order_id):
    return order_id


@shared_task
def tag(tags):
    return tags


@app.task(serializer="pickle")
def tag_pickled(tags):
    return tags


@shared_task(bind=True)
def run(self, payload):
    return payload


def not_a_task(x):
    return x
