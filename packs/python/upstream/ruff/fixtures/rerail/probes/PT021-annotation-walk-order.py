import pytest


@pytest.fixture
def annotated(request):
    x: request.addfinalizer(one) = request.addfinalizer(two)


@pytest.fixture
def alias(request):
    type X[T: request.addfinalizer(one), U: request.addfinalizer(two)] = request.addfinalizer(three)


@pytest.fixture
def subscripted(request):
    x: list[request.addfinalizer(one)] = request.addfinalizer(two)
