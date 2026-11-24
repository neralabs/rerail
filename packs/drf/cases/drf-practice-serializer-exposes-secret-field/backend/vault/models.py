# The project's import root is backend/: other modules import this one as `vault.models`.
from django.db import models


class TimestampedModel(models.Model):
    created_at = models.DateTimeField(auto_now_add=True)

    class Meta:
        abstract = True


class StorageMixin(TimestampedModel):
    access_key_id = models.CharField(max_length=128)
    secret_access_key = models.TextField()

    class Meta:
        abstract = True


class S3Storage(StorageMixin):
    bucket = models.CharField(max_length=200)
