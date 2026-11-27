from rest_framework import serializers


class StorageSerializer(serializers.ModelSerializer):
    pass


class RedactingSerializer(serializers.ModelSerializer):
    secure_fields = ["secret_access_key"]

    def to_representation(self, instance):
        data = super().to_representation(instance)
        for name in self.secure_fields:
            data.pop(name, None)
        return data
