from celery import shared_task

from proj import tasks


@shared_task
def helper_task(x):
    return tasks.price.delay(x).get()


def test_price():
    assert tasks.price.delay(2).get() == 4
