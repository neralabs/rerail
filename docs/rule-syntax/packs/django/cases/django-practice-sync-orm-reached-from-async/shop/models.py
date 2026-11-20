from django.db import models


class Book(models.Model):
    title = models.CharField(max_length=100)
    hits = models.IntegerField(default=0)

    def bump(self):
        self.hits += 1
        self.save()

    def save(self, *args, **kwargs):
        self.title = self.title.strip()
        super().save(*args, **kwargs)

    def label(self):
        return self.title.upper()
