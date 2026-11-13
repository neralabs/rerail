from django.db import models


class Fake(models.Model):
    name = models.CharField(max_length=10)

    def __init__(self, *args, **kwargs):
        # Test code is skipped.
        super().__init__(*args, **kwargs)
        self._old_name = self.name
