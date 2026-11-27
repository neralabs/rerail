from django.db import models


class Customer(models.Model):
    email = models.EmailField(db_index=True)
    name = models.CharField(max_length=100)


class Order(models.Model):
    customer = models.ForeignKey(Customer, on_delete=models.CASCADE)
    number = models.CharField(max_length=20, unique=True)


class Product(models.Model):
    price_cents = models.IntegerField(null=True)


class Tag(models.Model):
    order = models.ForeignKey(Order, on_delete=models.CASCADE)
    name = models.CharField(max_length=50)

    class Meta:
        unique_together = [("order", "name")]


class SiteSettings(models.Model):
    currency = models.CharField(max_length=3, null=True)
