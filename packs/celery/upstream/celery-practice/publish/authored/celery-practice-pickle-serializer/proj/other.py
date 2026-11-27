# A module not named for Celery: a bare `task_serializer` is not Celery's setting.
task_serializer = "pickle"
accept_content = ["pickle"]


class Exporter:
    serializer = "pickle"


def export(cache, value):
    cache.set("k", value, serializer="pickle")
    return {"serializer": "pickle"}


# Not Celery sends: a multiprocessing pool's apply_async and a kombu producer.
def offload(pool, fn, producer, body):
    pool.apply_async(fn, serializer="pickle")
    producer.publish(body, serializer="pickle")
