from rest_framework import serializers

from core.fields import ReadOnlyFieldsMixin


# A plain Serializer ignores its whole Meta: every field is declared, and read_only_fields reaches none.
class PostPlainSerializer(serializers.Serializer):
    title = serializers.CharField()
    slug = serializers.SlugField(read_only=True)

    class Meta:
        read_only_fields = [
            "title",  # caught
            "slug",
            "created",
        ]


# The same through a base input serializer of the run, whose Meta the subclass inherits.
class BaseInputSerializer(serializers.Serializer):
    class Meta:
        read_only_fields = ["owner"]


class TransferInputSerializer(BaseInputSerializer):
    owner = serializers.IntegerField()  # caught
    note = serializers.CharField()


# A plain serializer that consumes the field in create(): it is written on purpose.
class ImportInputSerializer(serializers.Serializer):
    source = serializers.URLField()

    class Meta:
        read_only_fields = ["source"]  # caught

    def create(self, validated_data):
        return validated_data["source"]


# extra_kwargs on a plain serializer: ignored like on a ModelSerializer's declared field.
class SearchInputSerializer(serializers.Serializer):
    query = serializers.CharField()

    class Meta:
        extra_kwargs = {"query": {"max_length": 100}}  # caught


# A mixin of the run applies read_only_fields itself.
class LockedInputSerializer(ReadOnlyFieldsMixin, serializers.Serializer):
    title = serializers.CharField()

    class Meta:
        read_only_fields = ["title"]
