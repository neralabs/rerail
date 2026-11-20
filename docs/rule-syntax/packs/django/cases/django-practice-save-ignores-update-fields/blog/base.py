from django.db import models


class Stamped(models.Model):
    touched_by = models.CharField(max_length=40, blank=True)

    class Meta:
        abstract = True
