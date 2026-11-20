from django.db import models

from shop.tasks import reindex_order


class Order(models.Model):
    email = models.EmailField()

    def save(self, *args, **kwargs):
        super().save(*args, **kwargs)
        reindex_order.delay(self)  # caught
        reindex_order.delay(self.pk)


class Report:
    def publish(self):
        reindex_order.delay(self)
