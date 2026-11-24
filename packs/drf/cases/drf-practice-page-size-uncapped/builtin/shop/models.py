from django.db import models


class Product(models.Model):
    name = models.CharField(max_length=100)


class Order(models.Model):
    product = models.ForeignKey(Product, on_delete=models.CASCADE)
