from shop.helpers import total


async def test_total():
    assert total() == 0
