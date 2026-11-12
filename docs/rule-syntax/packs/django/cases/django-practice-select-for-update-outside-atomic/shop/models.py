from django.db import models


class Hold(models.Model):
    seats = models.IntegerField(default=0)
