from django.contrib.auth.models import User
from rest_framework import serializers
from rest_framework.fields import ReadOnlyField
from taggit.serializers import TagListSerializerField

from accounts.models import Invite, Note, Profile, Project, Team
from core.fields import LockedField, OptionalField, ReadOnlyFieldsMixin
from core.serializers import AuditedSerializer, OwnedSerializer, StampedSerializer


# A declared, writable CharField listed read-only: any client can still set `username`.
class UserSerializer(serializers.ModelSerializer):
    username = serializers.CharField(max_length=150)
    email = serializers.EmailField()
    full_name = serializers.SerializerMethodField()
    date_joined = serializers.DateTimeField(read_only=True)
    last_login = ReadOnlyField()
    locked = LockedField()

    class Meta:
        model = User
        fields = ["id", "username", "email", "full_name", "date_joined", "last_login", "locked", "is_staff"]
        read_only_fields = [
            "id",
            "username",  # caught
            "full_name",
            "date_joined",
            "last_login",
            "locked",
            "is_staff",
        ]

    def get_full_name(self, obj):
        return obj.get_full_name()


# extra_kwargs on a declared password: write_only never applies, so the password is returned.
class SignupSerializer(serializers.ModelSerializer):
    password = serializers.CharField(max_length=128)
    email = serializers.EmailField(required=True)
    username = serializers.CharField(max_length=150, required=True)

    class Meta:
        model = User
        fields = ("username", "email", "password")
        extra_kwargs = {
            "password": {"write_only": True},  # caught
            "email": {"required": True},
            "username": dict(max_length=150, required=True),
        }


# One option repeats the declared call, the other does not; `team` is not declared, so DRF honours it.
class InviteSerializer(serializers.ModelSerializer):
    email = serializers.EmailField(required=True)

    class Meta:
        model = Invite
        fields = ["email", "team"]
        extra_kwargs = {"email": {"required": True, "allow_blank": False}, "team": {"required": False}}  # caught


# A HyperlinkedModelSerializer, names joined with `+`; the identity and hidden fields are read-only anyway.
class TeamSerializer(serializers.HyperlinkedModelSerializer):
    slug = serializers.SlugField()
    members = serializers.PrimaryKeyRelatedField(many=True, queryset=User.objects.all())
    owner = serializers.HiddenField(default=serializers.CurrentUserDefault())
    url = serializers.HyperlinkedIdentityField(view_name="team-detail")

    class Meta:
        model = Team
        fields = ("url", "slug", "members", "owner", "name")
        read_only_fields = ("url", "owner") + ("slug",)  # caught


# `slug = None` removes the declared field: the inherited read_only_fields applies to the model field again.
class TeamSummarySerializer(TeamSerializer):
    slug = None

    class Meta(TeamSerializer.Meta):
        fields = ("url", "name", "slug")


# A nested serializer: read-only with read_only=True, writable without.
class ProfileSerializer(serializers.ModelSerializer):
    user = UserSerializer(read_only=True)
    team = TeamSerializer()

    class Meta:
        model = Profile
        fields = ["user", "team", "bio"]
        read_only_fields = ["user", "team"]  # caught


# `owner` is declared on OwnedSerializer, in core/serializers.py.
class ProjectSerializer(OwnedSerializer):
    class Meta:
        model = Project
        fields = ["id", "name", "owner"]
        read_only_fields = ["owner"]  # caught


# The reverse: read_only_fields comes from AuditedSerializer.Meta (core/serializers.py), the field is declared here.
class NoteSerializer(AuditedSerializer):
    created_by = serializers.CharField(max_length=100)  # caught
    updated_at = serializers.DateTimeField(read_only=True)

    class Meta(AuditedSerializer.Meta):
        model = Note
        fields = ["id", "text", "created_by", "updated_at"]


# Inherits both the declaration and the Meta of NoteSerializer: reported there, not again here.
class NoteAdminSerializer(NoteSerializer):
    pass


class NoteDetailSerializer(NoteSerializer):
    class Meta(NoteSerializer.Meta):
        fields = ["id", "text", "created_by", "updated_at", "stamp"]


# StampedSerializer (core/serializers.py) is reported in its own module.
class StampedNoteSerializer(StampedSerializer):
    class Meta(StampedSerializer.Meta):
        model = Note


# A mixin applies read_only_fields to the declared fields itself.
class ProjectAdminSerializer(ReadOnlyFieldsMixin, serializers.ModelSerializer):
    name = serializers.CharField()

    class Meta:
        model = Project
        fields = ["id", "name"]
        read_only_fields = ["name"]


# A plain Serializer ignores its whole Meta: `name` stays writable.
class PlainSerializer(serializers.Serializer):
    name = serializers.CharField()

    class Meta:
        read_only_fields = ["name"]  # caught


# Fields named in Meta but not declared: DRF honours both options.
class TeamAdminSerializer(serializers.ModelSerializer):
    class Meta:
        model = Team
        fields = ["id", "name", "slug", "owner"]
        read_only_fields = ["slug"]
        extra_kwargs = {"owner": {"write_only": True}}


# A declared field with a default is already optional: `required: False` asks nothing it does not do.
class TaggedNoteSerializer(serializers.ModelSerializer):
    created_by = serializers.CharField(default="system")
    text = serializers.CharField()

    class Meta:
        model = Note
        fields = ["created_by", "text"]
        extra_kwargs = {
            "created_by": {"required": False},
            "text": {"required": False},  # caught
        }


# A serializer that refuses all input writes no field: read_only_fields has nothing to protect.
class NoteExportSerializer(serializers.ModelSerializer):
    text = serializers.CharField()

    class Meta:
        model = Note
        fields = ["id", "text"]
        read_only_fields = ["id", "text"]

    def validate(self, data):
        raise TypeError("this serializer is read-only")


# A field class from another package counts by its name; a run class that is no field does not.
class TaggedTeamSerializer(serializers.ModelSerializer):
    tags = TagListSerializerField()
    members = OptionalField(serializers.PrimaryKeyRelatedField, {"many": True})

    class Meta:
        model = Team
        fields = ["id", "tags", "members"]
        read_only_fields = [
            "tags",  # caught
            "members",
        ]
