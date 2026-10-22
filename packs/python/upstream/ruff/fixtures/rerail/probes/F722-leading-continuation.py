# Ruff reads a backslash-newline at the start of a string annotation as nothing.
a: '''\\
list[int]''' = [42]
b: '''\\
\\
int''' = 1
c: "\\\nint" = 1
# A trailing one, or an indented line after it, is a syntax error.
d: '''list[int]\\
''' = [42]
e: "\\\n int" = 1
