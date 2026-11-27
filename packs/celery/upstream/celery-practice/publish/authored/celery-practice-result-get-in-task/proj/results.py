from celery import shared_task
from celery.result import AsyncResult


class SafeAsyncResult(AsyncResult):
    def __init__(self, *args, expected_task_name, **kwargs):
        super().__init__(*args, **kwargs)
        self.expected_task_name = expected_task_name


def get_task_result(task_id, task_name):
    return SafeAsyncResult(task_id, expected_task_name=task_name)


def schedule_price(item_id):
    from proj.tasks import price

    return price.apply_async((item_id,), queue="pricing")


@shared_task
def follow_up(task_id, item_id):
    get_task_result(task_id, "proj.tasks.price").get()  # caught
    pending = schedule_price(item_id)
    return pending.get()  # caught


def status_view(request, task_id):
    # Web code: polling a result outside a worker is fine.
    result = get_task_result(task_id, "proj.tasks.price")
    if result.ready():
        return result.get()
    return None
