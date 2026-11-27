# Not Django: a store with its own bulk_create, in a file that does not import Django.


class Store:
    def bulk_create(self, items, ignore_conflicts=False):
        return items


def save_all(store: Store, items):
    store.bulk_create(items, ignore_conflicts=True)
    return [item.id for item in items]
