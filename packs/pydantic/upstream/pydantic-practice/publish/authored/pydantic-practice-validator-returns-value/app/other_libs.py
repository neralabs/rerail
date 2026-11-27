# Validators of other libraries, and a project decorator named like Pydantic's: none of them stores a return value.
import attrs
from marshmallow import Schema, fields, validates, validates_schema
from rest_framework import serializers

from app.validation import validator


@attrs.define
class Point:
    x: int = attrs.field()

    @x.validator
    def check_x(self, attribute, value):
        if value < 0:
            raise ValueError("negative")


class UserSchema(Schema):
    email = fields.Str()

    @validates("email")
    def validate_email(self, value):
        if not value:
            raise ValueError("required")

    @validates_schema
    def validate_all(self, data, **kwargs):
        if not data:
            raise ValueError("empty")


class UserSerializer(serializers.Serializer):
    email = serializers.EmailField()

    def validate_email(self, value):
        if value.endswith("@example.com"):
            raise serializers.ValidationError("taken")


class Settings:
    @validator("debug")
    def check_debug(cls, v):
        if v:
            raise ValueError("no debug in production")
