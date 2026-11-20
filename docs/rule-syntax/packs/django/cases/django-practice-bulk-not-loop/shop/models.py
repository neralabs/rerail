from django.db import models


class Book(models.Model):
    title = models.CharField(max_length=200)
    price = models.DecimalField(max_digits=8, decimal_places=2)
    hits = models.IntegerField(default=0)
