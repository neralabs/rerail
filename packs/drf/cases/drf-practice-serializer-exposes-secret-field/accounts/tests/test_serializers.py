from rest_framework import serializers

from accounts.models import User


class DumpUserSerializer(serializers.ModelSerializer):
    class Meta:
        model = User
        fields = "__all__"


def test_dump():
    assert DumpUserSerializer
