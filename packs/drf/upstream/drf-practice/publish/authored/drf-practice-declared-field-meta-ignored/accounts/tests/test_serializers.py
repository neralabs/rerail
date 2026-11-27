from django.contrib.auth.models import User
from rest_framework import serializers


class FixtureUserSerializer(serializers.ModelSerializer):
    username = serializers.CharField()

    class Meta:
        model = User
        fields = ["username"]
        read_only_fields = ["username"]
