from rest_framework import serializers
from rest_framework.serializers import ModelSerializer, SerializerMethodField

from storage.constants import BUCKET_FIELDS, BUCKET_PUBLIC_FIELDS, WRITE_ONLY
from storage.models import Bucket, Device, Document, Share

LOCAL_FIELDS = ("id",) + ("secret_access_key",)


# ---- reported -----------------------------------------------------------------------------------------------------


class BucketSerializer(serializers.ModelSerializer):
    class Meta:
        model = Bucket
        fields = "__all__"  # caught


# fields from a constant of another module: reported at the Meta value.
class BucketConstSerializer(serializers.ModelSerializer):
    class Meta:
        model = Bucket
        fields = BUCKET_FIELDS  # caught


# A constant of this module, built with `+`.
class BucketLocalSerializer(ModelSerializer):
    class Meta:
        model = Bucket
        fields = LOCAL_FIELDS  # caught


# A SerializerMethodField whose method returns the secret field.
class BucketMethodSerializer(serializers.ModelSerializer):
    sak = serializers.SerializerMethodField()  # caught

    class Meta:
        model = Bucket
        fields = ["id", "sak"]

    def get_sak(self, obj):
        return obj.secret_access_key


# The same through method_name= and an imported field class.
class BucketNamedMethodSerializer(ModelSerializer):
    credential = SerializerMethodField(method_name="read_secret")  # caught

    class Meta:
        model = Bucket
        fields = ["id", "credential"]

    def read_secret(self, bucket):
        return bucket.secret_access_key


# A public share-link token is reported: no name tells it from an API token (a gap, documented).
class ShareSerializer(serializers.ModelSerializer):
    class Meta:
        model = Share
        fields = ["id", "token"]  # caught


# ---- left alone ---------------------------------------------------------------------------------------------------


# `tokens` (plural, no suffix) names NLP tokens, not a secret.
class DocumentSerializer(serializers.ModelSerializer):
    class Meta:
        model = Document
        fields = ["id", "title", "tokens"]


# `access_key` without `secret` is a key id.
class BucketPublicSerializer(serializers.ModelSerializer):
    class Meta:
        model = Bucket
        fields = BUCKET_PUBLIC_FIELDS


# Plain names are not on the list (a gap, documented).
class DeviceSerializer(serializers.ModelSerializer):
    class Meta:
        model = Device
        fields = ["id", "key", "pin"]


# write-only through extra_kwargs, written out or as a module constant.
class BucketWriteSerializer(serializers.ModelSerializer):
    class Meta:
        model = Bucket
        fields = ["id", "secret_access_key"]
        extra_kwargs = {"secret_access_key": {"write_only": True}}


class BucketWriteConstSerializer(serializers.ModelSerializer):
    class Meta:
        model = Bucket
        fields = ["id", "secret_access_key"]
        extra_kwargs = WRITE_ONLY


class BucketWriteDictSerializer(serializers.ModelSerializer):
    class Meta:
        model = Bucket
        fields = ["id", "secret_access_key"]
        extra_kwargs = {"secret_access_key": dict(write_only=True)}


# A method field returning something else, and one left out of the output.
class BucketSafeMethodSerializer(serializers.ModelSerializer):
    masked = serializers.SerializerMethodField()

    class Meta:
        model = Bucket
        fields = ["id", "masked"]

    def get_masked(self, obj):
        return obj.access_key


# A plain Serializer declaring the field: plain serializers are out of scope (a gap, documented).
class BucketOutSerializer(serializers.Serializer):
    secret_access_key = serializers.CharField()
