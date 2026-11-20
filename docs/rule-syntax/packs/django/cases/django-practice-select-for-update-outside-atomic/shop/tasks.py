from celery import shared_task
from django.db import transaction

from .models import Hold


@shared_task
def expire(pk):
    hold = Hold.objects.select_for_update().get(pk=pk)  # caught
    hold.delete()


@shared_task
def expire_safe(pk):
    with transaction.atomic():
        hold = Hold.objects.select_for_update().get(pk=pk)
        hold.delete()


@shared_task
@transaction.atomic
def expire_decorated(pk):
    Hold.objects.select_for_update().get(pk=pk).delete()
