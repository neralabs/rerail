from django.shortcuts import get_object_or_404
from django.views import View

from shop import tasks
from shop.models import Order
from shop.tasks import email_user, ship_order


def ship(request, pk):
    order = Order.objects.get(pk=pk)
    ship_order.delay(order)  # caught
    ship_order.delay(order.pk)
    ship_order.delay(order=order)  # caught
    ship_order.apply_async((order,), countdown=10)  # caught
    ship_order.apply_async(args=[order], countdown=10)  # caught
    ship_order.apply_async(kwargs={"order": order})  # caught
    ship_order.apply_async(args=[order.pk])
    ship_order.s(order)  # caught
    tasks.ship_order.delay(order)  # caught
    email_user.delay(request.user)  # caught
    email_user.delay(request.user.pk)
    tasks.send_report.delay(order)
    fresh = Order(email="a@example.com")
    ship_order.delay(fresh)  # caught
    ship_order.delay(Order.objects.create(email="b@example.com"))  # caught
    found = get_object_or_404(Order, pk=pk)
    ship_order.delay(found)  # caught
    ids = Order.objects.filter(email="").values_list("pk", flat=True)
    ship_order.delay(ids)
    for each in Order.objects.filter(email=""):
        ship_order.delay(each)  # caught
        ship_order.delay(each.pk)
    first_pk = Order.objects.values_list("pk", flat=True).get(email="")
    ship_order.delay(first_pk)
    row = Order.objects.filter(email="").values("pk").order_by("pk").first()
    ship_order.delay(row)
    count = Order.objects.count()
    ship_order.delay(count)
    # A loop over values()/values_list() rows yields pks and dicts, not instances: the shape this rule asks for.
    for pk in Order.objects.filter(email="").values_list("pk", flat=True):
        ship_order.delay(pk)
    for line in Order.objects.filter(email="").values("pk", "email"):
        ship_order.delay(line)


class ShipView(View):
    def post(self, request):
        user = self.request.user
        email_user.delay(user)  # caught
        email_user.delay(self)
