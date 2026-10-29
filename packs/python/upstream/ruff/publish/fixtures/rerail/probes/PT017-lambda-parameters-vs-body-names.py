try:
    pass
except Exception as e:
    assert lambda e: 0
    assert lambda *, e: 0
    assert lambda x=e: 0
    assert lambda e=1: e
    assert lambda *e: 0
    assert lambda **e: 0
    assert lambda e, /: 0
    assert lambda x=lambda e: 0: 0
