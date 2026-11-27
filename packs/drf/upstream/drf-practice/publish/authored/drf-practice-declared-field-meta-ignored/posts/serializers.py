from rest_framework import serializers

from posts.constants import AUDIT_READONLY, OPTIONAL, READ_ONLY
from posts.models import Post

RO = {"read_only": True}
RW = {"read_only": False}
READONLY = ["author"]
TITLE_OPTIONS = {"title": {"max_length": 5}}


# read_only=True through `**` of a module-level dict, here or imported: the field is read-only.
class PostKwSerializer(serializers.ModelSerializer):
    slug = serializers.SlugField(**RO)
    summary = serializers.CharField(**READ_ONLY)

    class Meta:
        model = Post
        fields = ["id", "slug", "summary", "title"]
        read_only_fields = ["slug", "summary"]


# The `**` dict does not make it read-only.
class PostKwWritableSerializer(serializers.ModelSerializer):
    slug = serializers.SlugField(**RW)
    summary = serializers.CharField(**OPTIONAL)

    class Meta:
        model = Post
        fields = ["id", "slug", "summary"]
        read_only_fields = [
            "slug",  # caught
            "summary",  # caught
        ]


# read_only_fields through a module constant, here or imported: reported at the declaration, since the string is not
# in the serializer.
class PostConstSerializer(serializers.ModelSerializer):
    author = serializers.PrimaryKeyRelatedField(queryset=Post.objects.none())  # caught

    class Meta:
        model = Post
        fields = ["id", "author", "title"]
        read_only_fields = READONLY


class PostAuditSerializer(serializers.ModelSerializer):
    editor = serializers.PrimaryKeyRelatedField(queryset=Post.objects.none())  # caught
    summary = serializers.CharField()  # caught
    title = serializers.CharField()

    class Meta:
        model = Post
        fields = ["id", "editor", "summary", "title"]
        read_only_fields = [*AUDIT_READONLY, "id"]


# A constant spliced into a written list, and one joined with `+`.
class PostMixedSerializer(serializers.ModelSerializer):
    author = serializers.PrimaryKeyRelatedField(queryset=Post.objects.none())  # caught
    title = serializers.CharField()

    class Meta:
        model = Post
        fields = ["id", "author", "title"]
        read_only_fields = [
            *READONLY,
            "title",  # caught
        ]


class PostJoinedSerializer(serializers.ModelSerializer):
    slug = serializers.SlugField()

    class Meta:
        model = Post
        fields = ["id", "author", "slug"]
        read_only_fields = AUDIT_READONLY + ("slug",)  # caught


# extra_kwargs on a declared field: written out, through dict(), or a module constant.
class PostEkSerializer(serializers.ModelSerializer):
    title = serializers.CharField()

    class Meta:
        model = Post
        fields = ["id", "title"]
        extra_kwargs = {"title": {"max_length": 5}}  # caught


class PostEkDictSerializer(serializers.ModelSerializer):
    title = serializers.CharField()

    class Meta:
        model = Post
        fields = ["id", "title"]
        extra_kwargs = dict(title={"max_length": 5})  # caught


class PostEkConstSerializer(serializers.ModelSerializer):
    title = serializers.CharField()  # caught

    class Meta:
        model = Post
        fields = ["id", "title"]
        extra_kwargs = TITLE_OPTIONS


# __init__ makes the field read-only itself: the entry is dead but harmless.
class PostInitSerializer(serializers.ModelSerializer):
    title = serializers.CharField()

    class Meta:
        model = Post
        fields = ["id", "title"]
        read_only_fields = ["title"]

    def __init__(self, *a, **kw):
        super().__init__(*a, **kw)
        self.fields["title"].read_only = True


# The same through a loop over literal names, and in get_fields.
class PostInitLoopSerializer(serializers.ModelSerializer):
    title = serializers.CharField()
    slug = serializers.SlugField()

    class Meta:
        model = Post
        fields = ["id", "title", "slug"]
        read_only_fields = ["title", "slug"]

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        for name in ("title", "slug"):
            self.fields[name].read_only = True


class PostGetFieldsSerializer(serializers.ModelSerializer):
    summary = serializers.CharField()

    class Meta:
        model = Post
        fields = ["id", "summary"]
        read_only_fields = ["summary"]

    def get_fields(self):
        fields = super().get_fields()
        fields["summary"].read_only = True
        return fields


# A mixin of the run does it for its subclasses.
class LockSlugMixin:
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.fields["slug"].read_only = True


class PostLockedSerializer(LockSlugMixin, serializers.ModelSerializer):
    slug = serializers.SlugField()

    class Meta:
        model = Post
        fields = ["id", "slug"]
        read_only_fields = ["slug"]


# __init__ sets another option, or sets read_only to False: clients can still write the field.
class PostInitOtherSerializer(serializers.ModelSerializer):
    title = serializers.CharField()
    slug = serializers.SlugField()

    class Meta:
        model = Post
        fields = ["id", "title", "slug"]
        read_only_fields = [
            "title",  # caught
            "slug",  # caught
        ]

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.fields["title"].required = False
        self.fields["slug"].read_only = False


# Only a subclass sets it read-only: the base's own instances still accept it.
class PostBaseSerializer(serializers.ModelSerializer):
    summary = serializers.CharField()

    class Meta:
        model = Post
        fields = ["id", "summary"]
        read_only_fields = ["summary"]  # caught


class PostLockedSummarySerializer(PostBaseSerializer):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.fields["summary"].read_only = True


# A default of CurrentUserDefault() does not stop a client from sending another author.
class PostAuthorSerializer(serializers.ModelSerializer):
    author = serializers.PrimaryKeyRelatedField(queryset=Post.objects.none(), default=serializers.CurrentUserDefault())

    class Meta:
        model = Post
        fields = ["id", "author"]
        read_only_fields = ("author",)  # caught
