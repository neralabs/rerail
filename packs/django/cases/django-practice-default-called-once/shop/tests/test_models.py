import uuid

from django.db import models


class Fixture(models.Model):
    # Test code is skipped.
    key = models.UUIDField(default=uuid.uuid4())
