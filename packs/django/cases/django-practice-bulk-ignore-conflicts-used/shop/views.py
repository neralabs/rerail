from django.core import serializers
from django.core.mail import send_mail
from rest_framework.response import Response

from shop.models import Article, Invite, Issue, IssueSequence, Label, Tag
from shop.serializers import LabelSerializer
from shop.tasks import send_invite


def invite(emails):
    # A conflicting invite was not stored: the database keeps its old token, and this mails a new one.
    invites = [Invite(email=e, token=make_token(e)) for e in emails]
    invites = Invite.objects.bulk_create(invites, batch_size=10, ignore_conflicts=True)
    for item in invites:  # caught
        send_invite.delay(item.email, item.token)


def invite_by_mail(emails):
    invites = [Invite(email=e, token=make_token(e)) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for item in invites:  # caught
        send_mail("Join us", item.token, None, [item.email])


def label_ids(names):
    labels = [Label(name=n) for n in names]
    Label.objects.bulk_create(labels, ignore_conflicts=True)
    return [label.pk for label in labels]  # caught


def import_issues(titles):
    issues = Issue.objects.bulk_create([Issue(title=t) for t in titles], ignore_conflicts=True)
    IssueSequence.objects.bulk_create(
        [
            IssueSequence(issue=issue, sequence=i)
            for i, issue in enumerate(issues)  # caught
        ]
    )


def tag_article(article: Article, names):
    tags = Tag.objects.bulk_create([Tag(name=n) for n in names], ignore_conflicts=True)
    article.tags.add(*tags)  # caught


def tag_one_by_one(article: Article, names):
    tags = [Tag(name=n) for n in names]
    Tag.objects.bulk_create(tags, ignore_conflicts=True)
    for tag in tags:  # caught
        article.tags.add(tag)


def create_labels(request, data):
    created = Label.objects.bulk_create([Label(**d) for d in data], ignore_conflicts=True)
    return Response({"labels": LabelSerializer(created, many=True).data})  # caught


def export_labels(data):
    created = Label.objects.bulk_create([Label(**d) for d in data], ignore_conflicts=True)
    return {"created": serializers.serialize("json", created)}  # caught


async def count_new_labels(names):
    created = await Label.objects.abulk_create([Label(name=n) for n in names], ignore_conflicts=True)
    inserted = len(created)  # caught
    return inserted


def first_label(names):
    created = Label.objects.bulk_create([Label(name=n) for n in names], ignore_conflicts=True)
    return created[0].id  # caught


def label_names_and_ids(names):
    for label in Label.objects.bulk_create([Label(name=n) for n in names], ignore_conflicts=True):  # caught
        print(label.name, label.pk)


def refetch_by_pk(names):
    # Re-fetching by primary key does not help: the keys are unset, or name rows that do not exist.
    created = Label.objects.bulk_create([Label(name=n) for n in names], ignore_conflicts=True)
    return Label.objects.filter(pk__in=[c.pk for c in created])  # caught


def make_token(email):
    return email[::-1]
