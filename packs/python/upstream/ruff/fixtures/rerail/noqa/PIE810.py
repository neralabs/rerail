s = 'abc'
s.startswith('a') or s.startswith('b')

s = 'abc'
s.startswith('a') or s.startswith('b')  # noqa

s = 'abc'
s.startswith('a') or s.startswith('b')  # noqa: PIE810

s = 'abc'
s.startswith('a') or s.startswith('b')  # NOQA: PIE810

s = 'abc'
s.startswith('a') or s.startswith('b')  # noqa: E501
