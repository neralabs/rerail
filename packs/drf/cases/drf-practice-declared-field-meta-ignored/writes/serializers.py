from django.contrib.auth.models import User
from rest_framework import serializers

from writes.fields import SlugTagListField, TagListField, UpperCharField
from writes.models import Article, Bookmark


# linkding: `tag_names` is listed read-only, but create() consumes it: writes are intended, so the fix is to drop the
# entry. The Meta comes before the declaration.
class BookmarkSerializer(serializers.ModelSerializer):
    class Meta:
        model = Bookmark
        fields = ["id", "url", "title", "tag_names", "favicon_url"]
        read_only_fields = [
            "favicon_url",
            "tag_names",  # caught
        ]

    tag_names = serializers.ListField(child=serializers.CharField(), required=False)
    favicon_url = serializers.SerializerMethodField()

    def get_favicon_url(self, obj):
        return None

    def create(self, validated_data):
        tag_names = validated_data.pop("tag_names", [])
        bookmark = Bookmark.objects.create(**validated_data)
        bookmark.set_tags(tag_names)
        return bookmark


# update() reads it by subscript.
class ArticleOwnerSerializer(serializers.ModelSerializer):
    owner = serializers.PrimaryKeyRelatedField(queryset=User.objects.all())

    class Meta:
        model = Article
        fields = ["id", "title", "owner"]
        read_only_fields = ["owner"]  # caught

    def update(self, instance, validated_data):
        if "owner" in validated_data:
            instance.transfer_to(validated_data["owner"])
        return super().update(instance, validated_data)


# validate() reads it with .get, under another parameter name.
class ArticleEditorSerializer(serializers.ModelSerializer):
    editor = serializers.PrimaryKeyRelatedField(queryset=User.objects.all(), required=False)

    class Meta:
        model = Article
        fields = ["id", "title", "editor"]
        read_only_fields = ("editor",)  # caught

    def validate(self, values):
        if values.get("editor") is not None and not values["editor"].is_staff:
            raise serializers.ValidationError("Editors must be staff.")
        return values


# A validate_<field> method: the field is validated as input.
class ArticleSlugSerializer(serializers.ModelSerializer):
    slug = serializers.SlugField()

    class Meta:
        model = Article
        fields = ["id", "slug"]
        read_only_fields = ["slug"]  # caught

    def validate_slug(self, value):
        return value.lower()


# save() reads it from self.validated_data.
class ArticleStatusSerializer(serializers.ModelSerializer):
    status = serializers.CharField()

    class Meta:
        model = Article
        fields = ["id", "status"]
        read_only_fields = ["status"]  # caught

    def save(self, **kwargs):
        if self.validated_data.get("status") == "published":
            kwargs["published_by"] = self.context["request"].user
        return super().save(**kwargs)


# to_internal_value() reads it from the raw data.
class ArticleTitleSerializer(serializers.ModelSerializer):
    title = serializers.CharField()

    class Meta:
        model = Article
        fields = ["id", "title"]
        read_only_fields = ["title"]  # caught

    def to_internal_value(self, data):
        data = {**data, "title": data["title"].strip()}
        return super().to_internal_value(data)


# A field class of the run that parses input, its own to_internal_value or an inherited one; a field class that only
# changes output says nothing about writes.
class TaggedArticleSerializer(serializers.ModelSerializer):
    tags = TagListField()
    labels = SlugTagListField(required=False)
    shout = UpperCharField()

    class Meta:
        model = Article
        fields = ["id", "tags", "labels", "shout"]
        read_only_fields = [
            "tags",  # caught
            "labels",  # caught
            "shout",  # caught
        ]


# The server sets these, and clients can still forge them on update: create() stores into validated_data (a write,
# not a read), save() reads the view's keyword arguments, and .get on the context reads no input.
class ArticleCreatorSerializer(serializers.ModelSerializer):
    creation_user = serializers.PrimaryKeyRelatedField(queryset=User.objects.all(), required=False)
    published_by = serializers.PrimaryKeyRelatedField(queryset=User.objects.all(), required=False)

    class Meta:
        model = Article
        fields = ["id", "creation_user", "published_by"]
        read_only_fields = [
            "creation_user",  # caught
            "published_by",  # caught
        ]

    def create(self, validated_data):
        extra = {}
        if request := self.context.get("request"):
            extra["creation_user"] = request.user
        validated_data["creation_user"] = self.context.get("creation_user")
        return super().create({**validated_data, **extra})

    def save(self, **kwargs):
        if kwargs.get("published_by") is None:
            kwargs["published_by"] = self.context["request"].user
        return super().save(**kwargs)


# A validate_<field> for another field says nothing about this one.
class ArticleReviewSerializer(serializers.ModelSerializer):
    reviewer = serializers.PrimaryKeyRelatedField(queryset=User.objects.all())
    status = serializers.CharField()

    class Meta:
        model = Article
        fields = ["id", "reviewer", "status"]
        read_only_fields = ["reviewer"]  # caught

    def validate_status(self, value):
        return value


# A subclass whose own Meta does not list `reviewer` writes it: that is its own business, not the base's entry.
class ArticleReviewAdminSerializer(ArticleReviewSerializer):
    class Meta(ArticleReviewSerializer.Meta):
        read_only_fields = []

    def create(self, validated_data):
        validated_data["reviewer"].notify()
        return super().create(validated_data)


# The base lists `mentions` read-only and its create() consumes it; the subclass declares it.
class BaseCommentSerializer(serializers.ModelSerializer):
    class Meta:
        model = Article
        fields = ["id", "title", "mentions"]
        read_only_fields = ["mentions"]

    def create(self, validated_data):
        mentions = validated_data.pop("mentions", [])
        article = super().create(validated_data)
        article.notify(mentions)
        return article


class CommentSerializer(BaseCommentSerializer):
    mentions = serializers.ListField(child=serializers.CharField())  # caught

    class Meta(BaseCommentSerializer.Meta):
        pass


# Tandoor: a project base serializer overrides to_internal_value for every serializer; a nested serializer built on it
# parses input like any serializer, so it says nothing about this entry.
class NormalizingModelSerializer(serializers.ModelSerializer):
    def to_internal_value(self, data):
        return super().to_internal_value({k: v for k, v in data.items() if v != ""})


class OwnerSummarySerializer(NormalizingModelSerializer):
    class Meta:
        model = User
        fields = ["id", "username"]


class ArticleImportSerializer(NormalizingModelSerializer):
    owner = OwnerSummarySerializer()

    class Meta:
        model = Article
        fields = ["id", "title", "owner"]
        read_only_fields = ["id", "owner"]  # caught
