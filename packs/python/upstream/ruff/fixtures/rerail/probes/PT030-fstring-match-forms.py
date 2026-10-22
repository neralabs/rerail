import pytest


def test():
    pytest.warns(Warning, match=f"{{}}")
    pytest.warns(Warning, match=f"{''=}")
    pytest.warns(Warning, match=f"{''!r}")
    pytest.warns(Warning, match=f"{'':>3}")
    pytest.warns(Warning, match=f"{'',}")
    pytest.warns(Warning, match=f"{('')}")
    pytest.warns(Warning, match=f"{f''}")
    pytest.warns(Warning, match=f"{'' ''}")
    pytest.warns(Warning, match=f"" "")
    pytest.warns(Warning, match="" f"{None}")
    pytest.warns(Warning, match=f"{None!s:}")
    pytest.warns(Warning, match=F"")
    pytest.warns(Warning, match=fr"")
    pytest.warns(Warning, match=f'''{''}''')
    pytest.warns(Warning, match=f"{x}")
    pytest.warns(Warning, match=f"{'a'}")
    pytest.warns(Warning, match=f"{''}{None}{f'{None}'}")
    pytest.warns(Warning, match=f"{'' if x else ''}")
    pytest.warns(Warning, match=f"{b''}")
    pytest.warns(Warning, match=f"\n")
    pytest.warns(Warning, match=f"{'':{''}}")
    pytest.warns(Warning, match=f"{ '' }")
    pytest.warns(Warning, match=f"{
        ''
    }")
