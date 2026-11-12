from django.db import transaction
from django.http import JsonResponse

from .services import lock_hold, lock_safe, mid, mid_safe, outer


def bare_view(request, pk):
    hold = lock_hold(pk)
    mid(pk)
    return JsonResponse({"seats": hold.seats})


def atomic_view(request, pk):
    with transaction.atomic():
        lock_hold(pk)
        lock_safe(pk)
        mid_safe(pk)
        outer(pk)
    return JsonResponse({})
