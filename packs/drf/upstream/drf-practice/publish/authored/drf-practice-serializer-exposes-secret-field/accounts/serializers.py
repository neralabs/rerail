from django.contrib.auth import get_user_model
from django.contrib.auth.models import User as DjangoUser
from rest_framework import serializers

from accounts.models import ApiClient, Session, User
from core.fields import ObfuscatedField

UserModel = get_user_model()


# A custom user model deriving from AbstractUser: `password` (Django's) and `totp_secret` both go out.
class UserSerializer(serializers.ModelSerializer):
    class Meta:
        model = User
        fields = "__all__"  # caught


# `exclude` leaves the inherited api_token (core.models.CredentialModel) in the output.
class ApiClientSerializer(serializers.ModelSerializer):
    class Meta:
        model = ApiClient
        exclude = ["owner"]  # caught


# Listed explicitly.
class MeSerializer(serializers.ModelSerializer):
    class Meta:
        model = get_user_model()
        fields = (
            "id",
            "username",
            "password",  # caught
        )


# django.contrib.auth's own User, excluded fields not naming the password.
class StaffSerializer(serializers.ModelSerializer):
    class Meta:
        model = DjangoUser
        exclude = ("groups", "user_permissions")  # caught


# A module-level `UserModel = get_user_model()`.
class AdminUserSerializer(serializers.ModelSerializer):
    class Meta:
        model = UserModel
        fields = ["id", "email", "password"]  # caught


# A declared field that is not write-only still returns the model's value.
class ApiClientDetailSerializer(serializers.ModelSerializer):
    api_token = serializers.CharField(read_only=True)

    class Meta:
        model = ApiClient
        fields = "__all__"  # caught


# ---- left alone -------------------------------------------------------------------------------------------------


class UserCreateSerializer(serializers.ModelSerializer):
    class Meta:
        model = User
        fields = ["id", "username", "password", "totp_secret"]
        extra_kwargs = {"password": {"write_only": True}, "totp_secret": dict(write_only=True)}


class SignupSerializer(serializers.ModelSerializer):
    password = serializers.CharField(write_only=True)
    totp_secret = serializers.HiddenField(default="")

    class Meta:
        model = User
        fields = "__all__"


class PublicUserSerializer(serializers.ModelSerializer):
    class Meta:
        model = User
        fields = ["id", "username", "password_changed_at", "has_password"]


class SafeUserSerializer(serializers.ModelSerializer):
    class Meta:
        model = User
        exclude = ["password", "totp_secret"]


class MaskedApiClientSerializer(serializers.ModelSerializer):
    api_token = serializers.SerializerMethodField()

    class Meta:
        model = ApiClient
        fields = "__all__"

    def get_api_token(self, obj):
        return obj.api_token[:4] + "…"


class ObfuscatedApiClientSerializer(serializers.ModelSerializer):
    api_token = ObfuscatedField()

    class Meta:
        model = ApiClient
        fields = "__all__"


class SessionSerializer(serializers.ModelSerializer):
    class Meta:
        model = Session
        fields = "__all__"


# A plain Serializer: its fields are declared, not taken from a model.
class LoginSerializer(serializers.Serializer):
    username = serializers.CharField()
    password = serializers.CharField(write_only=True)
