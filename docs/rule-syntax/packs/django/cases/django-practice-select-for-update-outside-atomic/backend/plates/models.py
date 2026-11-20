from django.db import models


class Plate(models.Model):
    n = models.IntegerField(default=0)
