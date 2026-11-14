from functools import partial

from django.core.mail import EmailMessage, mail_admins, send_mail
from django.db import connection, connections, transaction
from django.http import HttpResponse
from django.utils.decorators import method_decorator
from django.views import View

from shop.models import Order
from shop.tasks import notify_after_commit, notify_later, notify_order, notify_when_committed


def place_order(request):
    order = Order.objects.create(email=request.POST["email"])
    notify_order.delay(order.pk)  # caught
    notify_order.apply_async(args=[order.pk], countdown=5)  # caught
    send_mail("Order", "Thanks", "shop@example.com", [order.email])  # caught
    EmailMessage("Order", "Thanks", to=[order.email]).send()  # caught
    message = EmailMessage("Order", "Copy", to=["ops@example.com"])
    message.send()  # caught
    notify_later(order)
    transaction.on_commit(partial(notify_order.delay, order.pk))
    transaction.on_commit(lambda: notify_order.delay(order.pk))
    transaction.on_commit(lambda: notify_after_commit(order))
    notify_order.delay_on_commit(order.pk)
    connection.on_commit(lambda: notify_order.delay(order.pk))
    connections["default"].on_commit(lambda: notify_order.delay(order.pk))
    return HttpResponse("ok")


def archive_order(request, pk):
    with transaction.atomic():
        notify_order.delay(pk)
    return HttpResponse("ok")


@transaction.atomic
def cancel_order(request, pk):
    notify_order.delay(pk)
    return HttpResponse("ok")


@transaction.non_atomic_requests
def export_orders(request):
    notify_order.delay(0)
    return HttpResponse("ok")


async def order_stream(request):
    notify_order.delay(0)
    return HttpResponse("ok")


def build_report(rows):
    mail_admins("Report", str(rows))


class OrderView(View):
    def post(self, request):
        notify_order.delay(1)  # caught
        self.alert_staff()
        return HttpResponse("ok")

    def alert_staff(self):
        mail_admins("Order", "New order")  # caught


@method_decorator(transaction.non_atomic_requests, name="dispatch")
class WebhookView(View):
    def post(self, request):
        notify_order.delay(2)
        return HttpResponse("ok")


class Mailer:
    def send_receipt(self, request):
        send_mail("Receipt", "Thanks", "shop@example.com", ["a@example.com"])


class ImportView(View):
    @classmethod
    def as_view(cls, **initkwargs):
        return transaction.non_atomic_requests(super().as_view(**initkwargs))

    def post(self, request):
        notify_order.delay(3)
        return HttpResponse("ok")


def upload(request):
    notify_order.delay(4)
    return HttpResponse("ok")


def place_or_alert(request):
    try:
        order = Order.objects.create(email=request.POST["email"])
    except KeyError:
        # The request fails and rolls back: an on_commit callback here would never run.
        mail_admins("Order failed", "no email")
        raise
    if not order.email:
        mail_admins("Order", "no email")  # caught
    return HttpResponse(str(order.pk))


def place_tracked(request):
    order = Order.objects.create(email=request.POST["email"])
    # The task id is stored on the row: on_commit returns nothing, so the call cannot simply move there.
    result = notify_order.apply_async(args=[order.pk])
    order.task_id = result.id
    order.save()
    return HttpResponse("ok")


def place_throttled(request, client):
    # `.delay()` on a parameter is not known to be a task.
    client.delay(2.0)
    # A task whose base class already sends through transaction.on_commit.
    notify_when_committed.delay(1)
    return HttpResponse("ok")
