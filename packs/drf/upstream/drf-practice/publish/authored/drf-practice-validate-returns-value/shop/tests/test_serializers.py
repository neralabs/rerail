from rest_framework import serializers


class FakeSerializer(serializers.Serializer):
    email = serializers.EmailField()

    def validate_email(self, value):
        pass

    def validate(self, attrs):
        pass
