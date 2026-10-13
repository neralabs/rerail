import httpx
from pathlib import Path
license = Path("LICENSE")
help = httpx.Client()
async def f():
    license.read_text()
    help.get("https://example.com")
