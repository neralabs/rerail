import backoff
import requests
from backoff import expo, on_exception, on_predicate

from .settings import settings


@backoff.on_exception(backoff.expo, requests.RequestException)  # caught
def get_page(url):
    return requests.get(url, timeout=5)


@backoff.on_predicate(backoff.fibo, lambda body: body is None, max_value=13)  # caught
def poll_job(job_id):
    return requests.get(f"/jobs/{job_id}", timeout=5).json().get("result")


@on_exception(  # caught
    expo,
    (requests.Timeout, requests.ConnectionError),
    jitter=backoff.full_jitter,
)
def post_event(event):
    return requests.post("/events", json=event, timeout=5)


@on_predicate(backoff.constant, interval=2)  # caught
def wait_ready():
    return requests.get("/ready", timeout=5).ok


# Bounded: max_tries or max_time is given.
@backoff.on_exception(backoff.expo, requests.RequestException, max_tries=5)
def get_page_bounded(url):
    return requests.get(url, timeout=5)


@backoff.on_exception(backoff.expo, requests.RequestException, max_time=60)
def get_page_for_a_minute(url):
    return requests.get(url, timeout=5)


@backoff.on_predicate(backoff.fibo, max_time=lambda: settings.poll_seconds)
def poll_job_bounded(job_id):
    return requests.get(f"/jobs/{job_id}", timeout=5).json().get("result")


BACKOFF = {"max_tries": 3, "jitter": None}


@on_exception(expo, requests.RequestException, **BACKOFF)
def get_page_shared(url):
    return requests.get(url, timeout=5)
