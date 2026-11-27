from rest_framework import serializers


def reject(message):
    raise serializers.ValidationError(message)


def reject_unless(ok, message):
    # Raises only on some paths: a call of it does not end the caller's path.
    if not ok:
        raise serializers.ValidationError(message)
