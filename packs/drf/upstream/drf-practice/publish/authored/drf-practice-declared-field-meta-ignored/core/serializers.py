from django.contrib.auth.models import User
from rest_framework import serializers


# Declares `owner`; subclasses in other modules list it in their own read_only_fields.
class OwnedSerializer(serializers.ModelSerializer):
    owner = serializers.PrimaryKeyRelatedField(queryset=User.objects.all())


# Lists `created_by` read-only for every subclass; a subclass that declares it gets nothing.
class AuditedSerializer(serializers.ModelSerializer):
    class Meta:
        read_only_fields = ["created_by", "updated_at"]


# Both in the same base: reported here, once, and not again in its subclasses.
class StampedSerializer(serializers.ModelSerializer):
    stamp = serializers.CharField()

    class Meta:
        fields = ["stamp"]
        read_only_fields = ["stamp"]  # caught
