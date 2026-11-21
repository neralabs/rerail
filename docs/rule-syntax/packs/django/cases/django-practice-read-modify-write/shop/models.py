from django.db import models


class Counted(models.Model):
    hits = models.PositiveIntegerField(default=0)

    class Meta:
        abstract = True


class Article(Counted):
    title = models.CharField(max_length=100)
    score = models.FloatField(default=0)
    balance = models.DecimalField(max_digits=10, decimal_places=2)
    likes = models.IntegerField(default=0)

    def like(self):
        self.likes += 1  # caught
        self.save(update_fields=["likes"])

    def bump(self, n):
        self.score = self.score + n  # caught
        self.save()

    def rename(self, suffix):
        self.title += suffix
        self.save()

    def count_only(self):
        self.likes += 1

    def save_first(self):
        self.save()
        self.likes += 1

    def later(self):
        def inner():
            self.save()
        self.likes += 1
        return inner


class Wallet(models.Model):
    amount = models.BigIntegerField(default=0)
