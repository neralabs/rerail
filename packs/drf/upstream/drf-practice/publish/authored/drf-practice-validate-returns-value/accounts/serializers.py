from rest_framework import serializers

from core import errors
from core.errors import reject, reject_unless


def _deny(message):
    if message:
        raise serializers.ValidationError(message)
    else:
        raise serializers.ValidationError("denied")


class AccountSerializer(serializers.Serializer):
    email = serializers.EmailField()
    code = serializers.CharField()
    handle = serializers.CharField()
    team = serializers.CharField()
    region = serializers.CharField()
    nickname = serializers.CharField()
    plan = serializers.CharField()

    default_error_messages = {"invalid": "Invalid code."}

    # Ends in a call of an imported function that always raises.
    def validate_email(self, value):
        if value.endswith("@example.com"):
            return value
        reject("bad domain")

    # DRF's Field.fail raises ValidationError.
    def validate_code(self, value):
        if value.isdigit():
            return value
        self.fail("invalid")

    # A module function of this file whose every path raises.
    def validate_handle(self, value):
        if value.isalnum():
            return value
        _deny("bad handle")

    # A method of the class whose every path raises.
    def validate_team(self, value):
        if value:
            return value
        self._reject_team()

    def _reject_team(self):
        raise serializers.ValidationError("team required")

    # Through the module object.
    def validate_region(self, value):
        if value in {"eu", "us"}:
            return value
        errors.reject("unknown region")

    # A helper that raises only on some paths: the value is lost when it returns.
    def validate_nickname(self, value):  # caught
        reject_unless(len(value) < 20, "too long")

    # `return None` in a field hook is a value written on purpose.
    def validate_plan(self, value):
        if value == "none":
            return None
        return value

    def validate(self, attrs):
        if not attrs.get("email"):
            return None  # caught
        return attrs


class FailingFieldSerializer(serializers.Serializer):
    code = serializers.CharField()

    def fail(self, key, **kwargs):
        # Overrides DRF's fail and does not raise.
        return None

    def validate_code(self, value):  # caught
        if value.isdigit():
            return value
        self.fail("invalid")
