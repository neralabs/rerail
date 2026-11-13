from django.db.models.signals import post_save
from django.dispatch import receiver


@receiver(post_save)
def connected_in_ready(sender, **kwargs):
    pass
