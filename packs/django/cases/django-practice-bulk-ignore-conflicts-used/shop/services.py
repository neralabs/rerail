import logging

from django.db import IntegrityError

from shop.models import Issue, Label

logger = logging.getLogger(__name__)


def refetch_by_unique_field(names):
    # The safe idiom: re-fetch the stored rows by the field that conflicts, then use those.
    labels = [Label(name=n) for n in names]
    Label.objects.bulk_create(labels, ignore_conflicts=True)
    labels = Label.objects.filter(name__in=[label.name for label in labels])
    return [label.pk for label in labels]


def read_before(names):
    labels = [Label(name=n) for n in names]
    wanted = [label.name for label in labels]
    Label.objects.bulk_create(labels, ignore_conflicts=True)
    return wanted


def batched(rows):
    batch = []
    for row in rows:
        batch.append(Label(name=row))
        if len(batch) >= 100:
            Label.objects.bulk_create(batch, ignore_conflicts=True)
            batch = []
    if batch:
        Label.objects.bulk_create(batch, ignore_conflicts=True)


def truth_only(names):
    created = Label.objects.bulk_create([Label(name=n) for n in names], ignore_conflicts=True)
    if created:
        logger.info("labels stored")


def upsert(names):
    # update_conflicts sets primary keys on PostgreSQL.
    created = Label.objects.bulk_create(
        [Label(name=n) for n in names], update_conflicts=True, unique_fields=["name"], update_fields=["color"]
    )
    return [label.pk for label in created]


def plain_insert(names):
    created = Label.objects.bulk_create([Label(name=n) for n in names])
    return [label.pk for label in created]


def explicit_false(names):
    created = Label.objects.bulk_create([Label(name=n) for n in names], ignore_conflicts=False)
    return [label.pk for label in created]


def own_values_only(issues):
    # Reads values the code set itself, not keys the database assigned.
    Issue.objects.bulk_create(issues, ignore_conflicts=True)
    return {issue.label_id for issue in issues}


def fallback(records):
    # The handler runs when the bulk insert failed, so nothing was inserted.
    try:
        Issue.objects.bulk_create(records, ignore_conflicts=True)
    except IntegrityError:
        for record in records:
            record.save()


def local_set(names):
    labels = [Label(name=n) for n in names]
    Label.objects.bulk_create(labels, ignore_conflicts=True)
    seen = set()
    for label in labels:
        seen.add(label)
    return seen


def count_processed(rows):
    # len() of the input list counts the objects processed, which it is.
    processed = 0
    batch = [Label(name=r) for r in rows]
    Label.objects.bulk_create(batch, ignore_conflicts=True)
    processed += len(batch)
    return processed
