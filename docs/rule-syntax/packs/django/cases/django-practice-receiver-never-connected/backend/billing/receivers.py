from django.core.signals import request_started
from django.dispatch import receiver


@receiver(request_started)
def imported_by_its_tail(sender, **kwargs):
    pass
