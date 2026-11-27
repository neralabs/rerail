from rest_framework import serializers


class ObfuscatedField(serializers.CharField):
    def to_representation(self, value):
        return "**********" if value else ""
