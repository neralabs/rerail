from django.db import models
from django.db.models.signals import post_save
from django.dispatch import receiver


class Book(models.Model):
    title = models.CharField(max_length=10)


@receiver(post_save, sender=Book)
def in_models_package(sender, **kwargs):
    pass
