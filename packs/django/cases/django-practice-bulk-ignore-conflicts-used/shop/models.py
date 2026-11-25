import uuid

from django.contrib.postgres.functions import RandomUUID
from django.db import models
from django.db.models import ForeignKey
from django_extensions.db.fields import AutoSlugField
from shortuuid.django_fields import ShortUUIDField
from tenants.models import TenantModel

from shop.base import BaseModel, MemberBase

GRANT_CONSTRAINTS = [models.UniqueConstraint(fields=["page", "grantee"], name="grant_unique")]


class Invite(models.Model):
    id = models.UUIDField(primary_key=True, default=uuid.uuid4)
    email = models.EmailField(unique=True)
    token = models.CharField(max_length=64)


class Label(models.Model):
    name = models.CharField(max_length=64, unique=True)
    color = models.CharField(max_length=7)


class Issue(models.Model):
    title = models.CharField(max_length=200, unique=True)
    label = models.ForeignKey(Label, null=True, on_delete=models.SET_NULL)


class IssueSequence(models.Model):
    issue = models.ForeignKey(Issue, on_delete=models.CASCADE)
    sequence = models.IntegerField()


class Tag(models.Model):
    name = models.CharField(max_length=64, unique=True)


class Article(models.Model):
    tags = models.ManyToManyField(Tag)


# Keyed models: the code fills the primary key.
class NaturalTag(models.Model):
    name = models.CharField(max_length=50, primary_key=True)


class Post(models.Model):
    tags = models.ManyToManyField(NaturalTag)


class TagLink(models.Model):
    tag = models.ForeignKey(NaturalTag, on_delete=models.CASCADE)


class Event(models.Model):
    id = models.UUIDField(primary_key=True, default=uuid.uuid4)
    key = models.CharField(max_length=40, unique=True)


class Line(models.Model):
    invite = models.ForeignKey(Invite, on_delete=models.CASCADE, null=True)
    backup_invite = ForeignKey(Invite, on_delete=models.CASCADE, null=True, related_name="+")
    note = models.CharField(max_length=40)


class Profile(models.Model):
    # The key is the label's: a related row the code names.
    owner = models.OneToOneField(Label, primary_key=True, on_delete=models.CASCADE)
    bio = models.TextField()


class Coupon(models.Model):
    code = models.UUIDField(primary_key=True, default=uuid.uuid4)
    slug = models.SlugField(unique=True)


# Models no row can conflict on: only a generated key is unique (BaseModel is in shop/base.py).
class Page(BaseModel):
    name = models.TextField(blank=True)
    live_objects = models.Manager()
    parent = models.ForeignKey("self", null=True, on_delete=models.CASCADE)

    class Meta:
        ordering = ("-created_at",)


class ProjectPage(BaseModel):
    page = models.ForeignKey(Page, on_delete=models.CASCADE)


class Note(models.Model):
    text = models.TextField()


class Ticket(models.Model):
    # A key the database generates.
    id = models.UUIDField(primary_key=True, db_default=RandomUUID())
    body = models.TextField()


# Models with a unique constraint besides the key, declared in a way the rule must still see.
class Member(MemberBase):
    role = models.IntegerField(default=5)


class Seat(BaseModel):
    owner = models.OneToOneField(Label, on_delete=models.CASCADE)


class Membership(BaseModel):
    label = models.ForeignKey(Label, on_delete=models.CASCADE)
    project = models.CharField(max_length=40)

    class Meta:
        constraints = [models.UniqueConstraint(fields=["label", "project"], name="membership_unique")]


class Tenant(TenantModel):
    # A base outside the run may declare unique fields the rule cannot see.
    title = models.TextField()


class Voucher(models.Model):
    # A third-party key class may make its own key: the rule cannot tell, so it reports.
    id = ShortUUIDField(primary_key=True)
    code = models.CharField(max_length=20, unique=True)


class SlugPage(BaseModel):
    # A unique field of a third-party field class.
    slug = AutoSlugField(populate_from="title", unique=True)
    title = models.TextField()


class Grant(BaseModel):
    page = models.ForeignKey(Page, on_delete=models.CASCADE)
    grantee = models.CharField(max_length=40)

    class Meta:
        # A name the rule does not follow: it counts as a unique constraint.
        constraints = GRANT_CONSTRAINTS
