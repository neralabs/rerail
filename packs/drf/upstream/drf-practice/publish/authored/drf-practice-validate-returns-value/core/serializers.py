from rest_framework import serializers


class CodedSerializer(serializers.Serializer):
    """A workspace base in another module: serializers in shop/ derive from it."""

    code = serializers.CharField()


class SlugSerializer(serializers.Serializer):
    """Its subclasses declare `slug`; DRF calls validate_slug on them."""

    def validate_slug(self, value):  # caught
        if " " in value:
            raise serializers.ValidationError("no spaces")


class PrevalidatedSerializer(serializers.Serializer):
    """Runs validate_owner() itself, before DRF's validation: a hook of this base."""

    owner = serializers.CharField()

    def is_valid(self, *, raise_exception=False):
        self.validate_owner(self.initial_data.get("owner"))
        return super().is_valid(raise_exception=raise_exception)

    def validate_owner(self, value):
        if not value:
            raise serializers.ValidationError("owner required")
