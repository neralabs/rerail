import httpx
import requests

from app.schemas import Event, Plain


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
