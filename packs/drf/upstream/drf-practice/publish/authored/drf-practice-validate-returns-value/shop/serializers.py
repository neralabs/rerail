import logging

from rest_framework import serializers
from rest_framework.exceptions import ValidationError

from core.serializers import CodedSerializer, PrevalidatedSerializer, SlugSerializer
from .models import Product

logger = logging.getLogger(__name__)


class ContactSerializer(serializers.Serializer):
    email = serializers.EmailField()
    name = serializers.CharField()
    phone = serializers.CharField()
    code = serializers.CharField()
    note = serializers.CharField()
    city = serializers.CharField()
    age = serializers.IntegerField()
    nick = serializers.CharField()
    title = serializers.CharField()
    zip = serializers.CharField()
    tags = serializers.CharField()

    def validate_email(self, value):  # caught
        if Product.objects.filter(name=value).exists():
            raise ValidationError("taken")

    def validate_name(self, value):
        if not value:
            return  # caught
        return value.strip()

    def validate_phone(self, value):  # caught
        if value.startswith("+"):
            return value
        elif value.startswith("00"):
            return "+" + value[2:]

    def validate_code(self, value):
        if not value.isdigit():
            raise serializers.ValidationError("digits only")
        return value

    def validate_note(self, value):
        if value:
            return value.strip()
        else:
            raise serializers.ValidationError("required")

    def validate_city(self, value):  # caught
        try:
            return value.title()
        except AttributeError:
            logger.warning("bad city %r", value)

    def validate_age(self, value):
        return None

    def validate_nick(self, value):
        raise NotImplementedError

    def validate_title(self, value, strict):
        value.strip()

    def validate_zip(self, value):
        while True:
            if len(value) >= 5:
                return value
            value = "0" + value

    def validate_tags(self, value):  # caught
        def split(raw):
            if not raw:
                return
            return raw.split(",")

        self.tag_list = split(value)

    def validate_unique_together(self, attrs):
        if attrs.get("email") == attrs.get("name"):
            raise ValidationError("email and name must differ")

    def validate(self, attrs):  # caught
        self.validate_unique_together(attrs)
        if attrs["age"] < 18:
            raise ValidationError("adults only")


class ProductSerializer(serializers.ModelSerializer):
    class Meta:
        model = Product
        fields = ["id", "name", "sku"]

    def validate_sku(self, value):  # caught
        value = value.upper()

    def validate_price(self, value):
        # price is not in Meta.fields: DRF never calls this.
        if value < 0:
            raise ValidationError("negative")

    def validate(self, attrs):
        if not attrs.get("name"):
            return  # caught
        return attrs


class LooseProductSerializer(serializers.ModelSerializer):
    class Meta:
        model = Product
        fields = "__all__"

    def validate_price(self, value):  # caught
        if value < 0:
            raise ValidationError("negative")

    def validate_discount(self, value):
        # Not a field of Product: a helper.
        if value > 50:
            raise ValidationError("too much")

    def validate(self, attrs):
        attrs = super().validate(attrs)
        if attrs["price"] > 1000:
            raise ValidationError("too expensive")
        return attrs


class ChildProductSerializer(ProductSerializer):
    class Meta(ProductSerializer.Meta):
        pass

    def validate_name(self, value):  # caught
        if value.lower() == "test":
            raise ValidationError("reserved")


class CodedProductSerializer(CodedSerializer):
    def validate_code(self, value):  # caught
        if len(value) != 6:
            raise ValidationError("six characters")


class ArticleSerializer(SlugSerializer):
    slug = serializers.SlugField()


class ImportSerializer(PrevalidatedSerializer):
    def validate_owner(self, value):
        if value == "root":
            raise ValidationError("not root")


class MatchSerializer(serializers.Serializer):
    kind = serializers.CharField()
    word = serializers.CharField()

    def validate_kind(self, value):  # caught
        match value:
            case "a":
                return "A"
            case "b":
                return "B"

    def validate_word(self, value):
        match value:
            case "":
                raise ValidationError("empty")
            case other:
                return other

    def validate(self, data):
        try:
            return super().validate(data)
        finally:
            logger.info("validated")


class NotASerializer:
    def validate_email(self, value):
        self.email = value.lower()

    def validate(self, attrs):
        self.attrs = attrs


class UpperField(serializers.CharField):
    def validate_empty_values(self, data):
        super().validate_empty_values(data)


from drf_writable_nested import WritableNestedModelSerializer  # noqa: E402


class StepSerializer(serializers.ModelSerializer):
    class Meta:
        model = Product
        fields = ["id", "name"]


class RecipeSerializer(WritableNestedModelSerializer):
    # A many=True nested field: drf-writable-nested saves it from the initial data, not validated_data.
    steps = StepSerializer(many=True)
    # A single nested field: a None in validated_data is saved as None.
    main_step = StepSerializer()

    class Meta:
        model = Product
        fields = ["id", "steps", "main_step"]

    def validate_steps(self, value):
        if len(value) > 10:
            raise ValidationError("too many steps")

    def validate_main_step(self, value):  # caught
        if not value:
            raise ValidationError("required")
