from rest_framework import serializers


# A run subclass of ReadOnlyField: read-only whatever Meta says.
class LockedField(serializers.ReadOnlyField):
    pass


# Applies Meta.read_only_fields to every field, declared ones too.
class ReadOnlyFieldsMixin:
    def get_fields(self):
        fields = super().get_fields()
        for name in getattr(self.Meta, "read_only_fields", ()):
            if name in fields:
                fields[name].read_only = True
        return fields


# Not a field: a placeholder a serializer mixin swaps for a real one at runtime.
class OptionalField:
    def __init__(self, serializer_class, serializer_kwargs=None):
        self.serializer_class = serializer_class
        self.serializer_kwargs = serializer_kwargs or {}
