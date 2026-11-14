from celery import shared_task
from django.db import transaction
from django.http import JsonResponse
from django.utils.decorators import method_decorator
from django.views import View

from .models import Hold
from .services import lock_hold, lock_safe, mid, mid_safe, outer


@shared_task
def bare_job(pk):
    # No caller: a task, an entry point the worker calls with no transaction open.
    hold = lock_hold(pk)
    mid(pk)
    return hold.seats


def atomic_view(request, pk):
    with transaction.atomic():
        lock_hold(pk)
        lock_safe(pk)
        mid_safe(pk)
        outer(pk)
    return JsonResponse({})


def reserve(request, pk):
    # settings.py turns ATOMIC_REQUESTS on: Django runs this view in a transaction.
    hold = Hold.objects.select_for_update().get(pk=pk)
    hold.seats -= 1
    hold.save()
    return JsonResponse({})


class Svc:
    @method_decorator(transaction.atomic)
    def go(self, pk):
        hold = Hold.objects.select_for_update().get(pk=pk)
        return hold.seats


@method_decorator(transaction.atomic, name="dispatch")
class ReserveAll:
    # dispatch() runs every handler of the class inside the transaction.
    def handle(self, pk):
        return Hold.objects.select_for_update().get(pk=pk)


@method_decorator(transaction.atomic, name="post")
class ReservePost:
    def post(self, pk):
        return Hold.objects.select_for_update().get(pk=pk)

    def put(self, pk):
        # Only post() is wrapped.
        return Hold.objects.select_for_update().get(pk=pk)  # caught
