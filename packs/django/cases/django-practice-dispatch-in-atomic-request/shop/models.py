from django.db import models


class Order(models.Model):
    email = models.EmailField()
