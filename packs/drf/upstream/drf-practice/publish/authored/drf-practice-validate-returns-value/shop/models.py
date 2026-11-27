from django.db import models


class Product(models.Model):
    name = models.CharField(max_length=100)
    sku = models.CharField(max_length=20)
    price = models.DecimalField(max_digits=8, decimal_places=2)
