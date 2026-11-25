import uuid

from django.db import models


class AuditModel(models.Model):
    created_at = models.DateTimeField(auto_now_add=True)
    deleted_at = models.DateTimeField(null=True)

    class Meta:
        abstract = True


class BaseModel(AuditModel):
    # A generated key: `unique=True` on the key itself is no conflict a caller can hit.
    id = models.UUIDField(default=uuid.uuid4, unique=True, editable=False, db_index=True, primary_key=True)

    class Meta:
        abstract = True


class MemberBase(BaseModel):
    email = models.CharField(max_length=255)

    class Meta:
        abstract = True
        unique_together = ["email", "deleted_at"]
