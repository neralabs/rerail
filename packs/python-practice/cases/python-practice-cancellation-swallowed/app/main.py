import asyncio

from app.safe import work


async def main():
    try:
        await work()
    except (KeyboardInterrupt, asyncio.CancelledError):
        print("shutting down")


if __name__ == "__main__":
    asyncio.run(main())
