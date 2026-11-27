import httpx
import requests

from app.schemas import Event, Invoice, Plain


def post_event(event: Event) -> None:
    requests.post("https://hooks.example.com/events", json=event.model_dump(), timeout=5)  # caught


def put_event(event: Event) -> None:
    with requests.Session() as session:
        session.put("https://hooks.example.com/events/1", json=event.model_dump())  # caught


async def send_event(event: Event) -> None:
    async with httpx.AsyncClient() as client:
        await client.post("https://hooks.example.com/events", json=event.model_dump())  # caught


def sync_event(event: Event) -> None:
    client = httpx.Client()
    client.patch("https://hooks.example.com/events/1", json=event.model_dump())  # caught
    httpx.post(
        "https://hooks.example.com/events",
        json=event.model_dump(),  # caught
    )


# ---- left alone
def json_mode(event: Event) -> None:
    requests.post("https://hooks.example.com/events", json=event.model_dump(mode="json"))


def body_as_string(event: Event) -> None:
    httpx.post("https://hooks.example.com/events", content=event.model_dump_json())


def unknown_client(api_client, event: Event) -> None:
    api_client.post("/api/events", json=event.model_dump())


def json_native(plain: Plain) -> None:
    requests.post("https://hooks.example.com/plain", json=plain.model_dump())


# ---- reported: a client the class stores on self, or a subclass of a client class
class Pusher:
    def __init__(self) -> None:
        self.client = httpx.Client()

    def push(self, invoice: Invoice) -> None:
        self.client.post("/invoices", json=invoice.model_dump())  # caught

    def push_local(self, event: Event) -> None:
        client = self.client
        client.post("/events", json=event.model_dump())  # caught


class ApiClient(httpx.Client):
    pass


def subclass_client(event: Event) -> None:
    ApiClient(base_url="https://api.example.com").post("/events", json=event.model_dump())  # caught


# ---- left alone: an injected client, a subclass that swaps it, a client that overrides request()
class Injected:
    def __init__(self, client: httpx.Client | None = None) -> None:
        self.client = client or httpx.Client()

    def push(self, event: Event) -> None:
        self.client.post("/events", json=event.model_dump())


class BasePusher:
    def __init__(self) -> None:
        self.client = httpx.Client()

    def push(self, event: Event) -> None:
        self.client.post("/events", json=event.model_dump())


class RecordingPusher(BasePusher):
    def __init__(self) -> None:
        self.client = RecordingClient()


class RecordingClient:
    def post(self, url: str, json=None) -> None:
        self.sent = (url, json)


class SignedClient(httpx.Client):
    def request(self, method, url, **kwargs):
        kwargs.setdefault("headers", {})["x-signature"] = "..."
        return super().request(method, url, **kwargs)


def signed_client(event: Event) -> None:
    SignedClient().post("/events", json=event.model_dump())
