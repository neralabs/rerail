from django.db.models.signals import post_delete, post_save, pre_save
from django.dispatch import receiver

from .models import Label, Plain, Tag


@receiver(post_save, sender=Tag)
def tag_saved(sender, instance, **kwargs):
    Tag.objects.filter(pk=instance.pk).update(name=instance.name.lower())


@receiver([pre_save, post_save], sender=Plain)
def plain_saved(sender, instance, **kwargs):
    pass


@receiver(post_delete, sender=Label)
def label_deleted(sender, instance, **kwargs):
    pass


def label_saved(sender, instance, **kwargs):
    # The receiver itself writes in bulk, on purpose: no recursion.
    Label.objects.filter(pk=instance.pk).update(name=instance.name.lower())


pre_save.connect(label_saved, sender=Label)
