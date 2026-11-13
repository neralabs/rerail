from django.db.models.signals import post_delete
from django.dispatch import receiver


@receiver(post_delete)  # caught
def never_imported(sender, **kwargs):
    pass


def helper():
    pass
