from django.dispatch import receiver
from django.db.models.signals import pre_save


@receiver(pre_save)
def imported_absolute(sender, **kwargs):
    pass
