from django.dispatch import receiver
from django.db.models.signals import pre_delete


@receiver(pre_delete)
def imported_from_package(sender, **kwargs):
    pass
