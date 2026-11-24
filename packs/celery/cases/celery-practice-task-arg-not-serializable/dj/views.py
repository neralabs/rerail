from django.http import JsonResponse

from proj.models import Order
from proj.tasks import ship, tag


def view(request, pk):
    order = Order(id=pk)
    ship.delay(order)
    tag.delay({request.GET["tag"]})  # caught
    return JsonResponse({})
