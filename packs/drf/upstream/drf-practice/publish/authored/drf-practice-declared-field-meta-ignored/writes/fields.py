from rest_framework import serializers


# A custom field that parses input: the point of declaring it is to accept writes.
class TagListField(serializers.ListField):
    child = serializers.CharField()

    def to_internal_value(self, data):
        return [name.strip().lower() for name in super().to_internal_value(data)]


# A subclass inherits the parser.
class SlugTagListField(TagListField):
    pass


# A custom field that only changes output: nothing says writes are intended.
class UpperCharField(serializers.CharField):
    def to_representation(self, value):
        return super().to_representation(value).upper()
