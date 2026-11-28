from shop.models import Order
from shop.tasks import ship_order


def test_ship():
    order = Order.objects.create(email="a@example.com")
    ship_order.delay(order)
