from pytest.mark import foo
from pytest.mark.sub import bar


@foo()
def marker_import_flagged(): ...


@bar()
def deeper_import_not_a_marker(): ...
