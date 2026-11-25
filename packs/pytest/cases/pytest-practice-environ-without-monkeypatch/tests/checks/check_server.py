import os
import subprocess


# A check script that lives under tests/ but is no test: run by hand, it starts a server in its own process.
def check_server_starts():
    os.environ["PORT"] = "8765"
    subprocess.run(["uvicorn", "shop.asgi:app"], check=True)


if __name__ == "__main__":
    check_server_starts()
