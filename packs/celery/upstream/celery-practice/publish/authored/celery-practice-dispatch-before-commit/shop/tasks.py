from celery import shared_task

from shop.celery import app


@app.task
def ship(order_id):
    return order_id


@shared_task
def notify(order_id):
    return order_id
