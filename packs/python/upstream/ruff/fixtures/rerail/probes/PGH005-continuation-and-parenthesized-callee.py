(  # c
    m.assert_called  # c2
)
assert (  # c
    m.not_called()  # c2
)
assert(m.not_called())
assert(m.not_called)
assert (m.not_called)  # x
assert m.not_called() , "x" "y"
assert m.not_called() if 1 else 2
assert a.not_called() is b.not_called
assert m.not_called() \
    , "msg"
assert \
    m.not_called()
m.\
    assert_called
(m.
 assert_called)
x = 1; m.assert_called; y = 2
if x: m.assert_called; m.assert_called_once
assert (m.not_called)()
assert (m.not_called)(1, 2)  # c
