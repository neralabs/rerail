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
