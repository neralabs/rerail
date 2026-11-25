from vault.api.base import RedactingSerializer, StorageSerializer
from vault.models import S3Storage


# The model and the base serializer resolve through the project's import root (backend/).
class S3StorageSerializer(StorageSerializer):
    class Meta:
        model = S3Storage
        fields = "__all__"  # caught


class S3StorageRedactedSerializer(RedactingSerializer):
    class Meta:
        model = S3Storage
        fields = "__all__"


# The model through the package re-export (`vault/__init__.py`) and the import root.
from vault import S3Storage as ReexportedStorage  # noqa: E402


class S3StorageExportSerializer(StorageSerializer):
    class Meta:
        model = ReexportedStorage
        fields = ["bucket", "secret_access_key"]  # caught
