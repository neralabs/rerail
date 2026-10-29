import pytest


@pytest.mark.asyncio(reason='''it's # here''')  # c
@pytest.fixture
async def triple_single():
    pass


@pytest.fixture
@pytest.mark.asyncio(reason="""a"b # c""")
async def triple_double():
    pass
