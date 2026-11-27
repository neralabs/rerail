# The package re-exports its models; callers import them as `vault.S3Storage`.
from .models import S3Storage

__all__ = ["S3Storage"]
