from proj.models import Order
from proj.tasks import ship, tag


def test_ship():
    ship.delay(Order(id=1))
    tag.delay({"a"})
