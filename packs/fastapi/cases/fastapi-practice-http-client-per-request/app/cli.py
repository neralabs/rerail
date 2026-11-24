import httpx

from .services import export_all


def main():
    with httpx.Client() as client:
        print(client.get("https://example.com").status_code)
    export_all(["https://example.com"])


if __name__ == "__main__":
    main()
