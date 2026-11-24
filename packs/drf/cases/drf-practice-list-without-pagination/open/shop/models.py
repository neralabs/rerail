from django.db import models


class Customer(models.Model):
    user = models.ForeignKey("auth.User", on_delete=models.CASCADE)


class Order(models.Model):
    customer = models.ForeignKey(Customer, on_delete=models.CASCADE)
    created = models.DateTimeField(auto_now_add=True)


class Invoice(models.Model):
    order = models.ForeignKey(Order, on_delete=models.CASCADE)


class Tag(models.Model):
    owner = models.ForeignKey("auth.User", related_name="tags", on_delete=models.CASCADE)
    name = models.CharField(max_length=40)
