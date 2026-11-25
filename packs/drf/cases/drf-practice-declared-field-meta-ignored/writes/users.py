from django.contrib.auth.models import Group, User
from rest_framework import serializers


# InvenTree's user serializers. UserSerializer lists two declared fields read-only. ExtendedUserSerializer, the staff
# edit serializer, extends the Meta and so inherits the entry, and validates `email` as input: that one is written on
# purpose. Nothing in the run writes `username` on purpose.
class UserSerializer(serializers.ModelSerializer):
    class Meta:
        model = User
        fields = ["pk", "username", "first_name", "email"]
        read_only_fields = [
            "username",  # caught
            "email",  # caught
        ]

    username = serializers.CharField(max_length=150)
    first_name = serializers.CharField(allow_blank=True)
    email = serializers.EmailField(allow_blank=True)


class ExtendedUserSerializer(UserSerializer):
    class Meta(UserSerializer.Meta):
        fields = [*UserSerializer.Meta.fields, "groups", "is_staff", "is_superuser"]
        read_only_fields = [*UserSerializer.Meta.read_only_fields, "groups"]

    groups = serializers.PrimaryKeyRelatedField(many=True, read_only=True)
    is_staff = serializers.BooleanField(required=False)
    is_superuser = serializers.BooleanField(required=False)

    def validate_is_superuser(self, value):
        if not self.context["request"].user.is_superuser:
            raise serializers.ValidationError("Only a superuser can adjust this field")
        return value

    def validate_email(self, value):
        if User.objects.filter(email__iexact=value).exists():
            raise serializers.ValidationError("A user with this email address already exists")
        return value


# The create serializer means to block both flags, and its Meta's entry is its own: the base that validates
# `is_superuser` does not list it, so its validator says nothing about this entry.
class UserCreateSerializer(ExtendedUserSerializer):
    class Meta(ExtendedUserSerializer.Meta):
        # Prevent creation of users with superuser or staff permissions
        read_only_fields = [
            "groups",
            "is_staff",  # caught
            "is_superuser",  # caught
        ]

    def create(self, validated_data):
        groups = Group.objects.filter(name="default")
        user = super().create(validated_data)
        user.groups.set(groups)
        return user
