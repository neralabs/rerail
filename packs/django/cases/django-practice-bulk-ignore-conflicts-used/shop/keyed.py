from shop.models import Coupon, Event, NaturalTag, Post, Profile, TagLink
from shop.models import Event as Ev


def tag_post(post: Post, names):
    # NaturalTag's key is its name: conflict or not, each object names the stored row.
    tags = [NaturalTag(name=n) for n in names]
    NaturalTag.objects.bulk_create(tags, ignore_conflicts=True)
    post.tags.add(*tags)
    for t in tags:
        TagLink(tag_id=t.pk).save()


def record(keys):
    # The key is given: a conflict on it means the row with that key exists.
    events = [Event(id=k, key=str(k)) for k in keys]
    Event.objects.bulk_create(events, ignore_conflicts=True)
    return [e.id for e in events]


def record_by_pk(keys):
    events = [Event(pk=k, key=str(k)) for k in keys]
    Event.objects.bulk_create(events, ignore_conflicts=True)
    return [e.pk for e in events]


def record_aliased(keys):
    events = [Ev(id=k, key=str(k)) for k in keys]
    Ev.objects.bulk_create(events, ignore_conflicts=True)
    return [e.id for e in events]


def record_assigned(keys):
    events = []
    for k in keys:
        event = Event(key=str(k))
        event.pk = k
        events.append(event)
    Event.objects.bulk_create(events, ignore_conflicts=True)
    return [e.pk for e in events]


def coupons(codes):
    # Coupon's key is `code`; the code gives it.
    items = [Coupon(code=c, slug=str(c)) for c in codes]
    Coupon.objects.bulk_create(items, ignore_conflicts=True)
    return [item.pk for item in items]


def profiles(labels):
    # A one-to-one key: the label's row names the profile's.
    items = [Profile(owner=label, bio="") for label in labels]
    Profile.objects.bulk_create(items, ignore_conflicts=True)
    for item in items:
        TagLink(tag_id=item.pk)


def record_generated(keys):
    # The key comes from the default: a conflict on `key` leaves a uuid that names no row.
    events = [Event(key=str(k)) for k in keys]
    Event.objects.bulk_create(events, ignore_conflicts=True)
    return [e.id for e in events]  # caught


def count_tags(names):
    # Keyed or not, conflicting objects are skipped: len() still counts objects, not rows inserted.
    created = NaturalTag.objects.bulk_create([NaturalTag(name=n) for n in names], ignore_conflicts=True)
    return {"created": len(created)}  # caught
