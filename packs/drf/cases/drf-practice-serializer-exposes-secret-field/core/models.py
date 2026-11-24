from django.db import models


class CredentialModel(models.Model):
    """Abstract base: every API client holds a token."""

    api_token = models.CharField(max_length=64, unique=True)
    token_expires = models.DateTimeField(null=True)
    tokens_used = models.IntegerField(default=0)

    class Meta:
        abstract = True
