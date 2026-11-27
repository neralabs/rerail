from django.db.models.signals import post_save
from django.dispatch import Signal

from shop.models import Invite, IssueSequence

invite_created = Signal()


def attach_lines(emails, lines):
    # A skipped invite is not the stored row: the line points at a row that does not exist.
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for inv, line in zip(invites, lines):  # caught
        line.invite = inv
        line.save()


def attach_one(emails, line):
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for inv in invites:  # caught
        line.invite = inv
        line.save()


def zip_second(emails, lines):
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for line, inv in zip(lines, invites):  # caught
        line.backup_invite = inv


def enumerate_sorted(emails):
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for n, inv in enumerate(sorted(invites, key=lambda i: i.email)):  # caught
        IssueSequence(issue=inv, sequence=n)


def copied_and_sliced(emails):
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    first = [inv.pk for inv in list(invites)]  # caught
    rest = [inv.pk for inv in invites[10:]]  # caught
    return first, rest


def announce(emails):
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for inv in invites:  # caught
        post_save.send(sender=Invite, instance=inv, created=True)


def announce_custom(emails):
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for inv in reversed(invites):  # caught
        invite_created.send_robust(sender=Invite, invite=inv)


def other_side_of_zip(emails, issues):
    # The zip's other list holds the code's own rows.
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for inv, issue in zip(invites, issues):
        IssueSequence.objects.create(issue=issue, sequence=1)
        issue.title = inv.email


def plain_attribute(emails, cache):
    # `latest` is no foreign key of any model.
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for inv in invites:
        cache.latest = inv


def not_a_signal(emails, outbox):
    # A send without sender= is no signal: mail-like sends are matched by lib/django, not here.
    invites = [Invite(email=e, token=e) for e in emails]
    Invite.objects.bulk_create(invites, ignore_conflicts=True)
    for inv in invites:
        outbox.send(inv)
