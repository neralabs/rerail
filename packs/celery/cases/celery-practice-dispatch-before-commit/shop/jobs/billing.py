from celery import Task

from shop.celery import app


def _refund(order_id):
    return order_id


refund = app.task(_refund, name="shop.refund")


class RefundTask(Task):
    name = "shop.refund_task"

    def run(self, order_id):
        return order_id


refund_task = RefundTask()
