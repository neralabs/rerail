from django.db import models


class Item(models.Model):
    sku = models.CharField(max_length=40)
