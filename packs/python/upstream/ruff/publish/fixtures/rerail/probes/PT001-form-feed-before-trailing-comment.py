import pytest


@pytest.fixture()  # a
def a(): ...


@pytest.fixture("x")# b
def b(): ...


class K:
	@pytest.fixture(scope="function")
	def c(self): ...

	@pytest.fixture()	# d
	def d(self): ...
