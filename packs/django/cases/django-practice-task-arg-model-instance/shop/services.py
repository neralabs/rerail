from shop.models import Order
from shop.tasks import ship_order


def make_order():
    return Order(email="x")


def fetch(pk):
    return Order.objects.get(pk=pk)


def order_id(pk):
    return fetch(pk).pk


def use(pk):
    made = make_order()
    ship_order.delay(made)  # caught
    ship_order.delay(make_order())  # caught
    ship_order.delay(Order(email="z"))  # caught
    ship_order.delay(order_id(pk))


def resend(order: Order, note: str, count: int):
    ship_order.delay(order)  # caught
    ship_order.delay(note)
    ship_order.delay(count)


def resend_untyped(order):
    ship_order.delay(order)
