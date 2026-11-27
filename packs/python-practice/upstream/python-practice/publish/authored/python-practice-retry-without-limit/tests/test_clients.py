import time

from tenacity import retry, wait_fixed


@retry(wait=wait_fixed(0.1))
def flaky_fixture(client):
    return client.get("/items")


def test_eventually_up(client):
    while True:
        try:
            response = client.get("/health")
            break
        except ConnectionError:
            time.sleep(0.1)
    assert response.ok
