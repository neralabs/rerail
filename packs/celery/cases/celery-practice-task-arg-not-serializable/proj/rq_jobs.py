from multiprocessing.pool import ThreadPool

from rq.decorators import job

POOL = ThreadPool(2)


@job("default")
def rq_tag(tags):
    return tags


def compute(tags):
    return tags
