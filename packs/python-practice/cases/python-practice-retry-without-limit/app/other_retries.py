import requests
from celery import shared_task
from requests.adapters import HTTPAdapter
from urllib3.util import Retry


def retry(func):
    return func


@retry
def local_retry_decorator():
    return 1


def session():
    s = requests.Session()
    s.mount("https://", HTTPAdapter(max_retries=Retry(total=None, backoff_factor=1)))
    return s


@shared_task(bind=True)
def sync_account(self, account_id):
    try:
        return requests.get(f"/accounts/{account_id}", timeout=5)
    except requests.RequestException as exc:
        raise self.retry(exc=exc)
