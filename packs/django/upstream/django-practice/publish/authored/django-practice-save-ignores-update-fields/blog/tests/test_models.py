from django.db import models


class Fixture(models.Model):
    slug = models.SlugField()

    def save(self, *args, **kwargs):
        # Test code is skipped.
        self.slug = "x"
        super().save(*args, **kwargs)
