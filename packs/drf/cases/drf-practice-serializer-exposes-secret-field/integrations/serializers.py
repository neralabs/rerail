from rest_framework import serializers

from integrations.models import MailAccount, Provider, Webhook


class WebhookSerializer(serializers.ModelSerializer):
    class Meta:
        model = Webhook
        fields = [
            "id",
            "url",
            "secret_santa",
            "commit_hash",
            "signing_secret",  # caught
        ]


# Meta inherited from WebhookSerializer.Meta, fields extended.
class WebhookDetailSerializer(WebhookSerializer):
    class Meta(WebhookSerializer.Meta):
        fields = WebhookSerializer.Meta.fields + [
            "private_key",  # caught
            "password_hash",  # caught
        ]


# A custom model field class (EncryptedTextField) still holds a secret once decrypted.
class MailAccountSerializer(serializers.ModelSerializer):
    class Meta:
        model = MailAccount
        fields = "__all__"  # caught


class ProviderSerializer(serializers.ModelSerializer):
    llm_api_key = serializers.CharField(write_only=True)

    class Meta:
        model = Provider
        exclude = ["api_token"]  # caught


# ---- left alone -------------------------------------------------------------------------------------------------


class BaseWebhookSerializer(serializers.ModelSerializer):
    def to_representation(self, instance):
        data = super().to_representation(instance)
        data.pop("signing_secret", None)
        data.pop("private_key", None)
        data.pop("password_hash", None)
        return data


class WebhookListSerializer(BaseWebhookSerializer):
    class Meta:
        model = Webhook
        fields = "__all__"


class WebhookFingerprintSerializer(serializers.ModelSerializer):
    signing_secret = None
    password_hash = None
    private_key = serializers.CharField(source="public_key_fingerprint", read_only=True)

    class Meta:
        model = Webhook
        fields = "__all__"


class MailAccountWriteSerializer(serializers.ModelSerializer):
    class Meta:
        model = MailAccount
        fields = "__all__"
        extra_kwargs = {
            "password": {"write_only": True},
            "oauth_credentials": {"write_only": True},
        }


class ProviderPublicSerializer(serializers.ModelSerializer):
    class Meta:
        model = Provider
        fields = ["id", "name", "token_expires", "tokens_used"]


class SecureFieldsMixin:
    secure_fields = ["llm_api_key", "client_secret", "api_token"]

    def to_representation(self, instance):
        result = super().to_representation(instance)
        for attr in self.secure_fields:
            result.pop(attr, None)
        return result


class ProviderAdminSerializer(SecureFieldsMixin, serializers.ModelSerializer):
    class Meta:
        model = Provider
        fields = "__all__"


# A to_representation that does not drop the secret: the string in Meta does not count.
class ProviderExportSerializer(serializers.ModelSerializer):
    class Meta:
        model = Provider
        fields = ["id", "name", "llm_api_key"]  # caught

    def to_representation(self, instance):
        data = super().to_representation(instance)
        data["name"] = data["name"].title()
        return data
