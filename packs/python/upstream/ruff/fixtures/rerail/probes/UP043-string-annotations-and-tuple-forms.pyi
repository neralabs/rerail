from typing import AsyncGenerator, Generator, List, Optional

a: Optional["Generator[int, None]"]
b: "List['Generator[int, None]']"
c: ("Generator[int, None, None]" "")
d: "Generator[int, None, \x4eone]"
e: Generator[(int, None),]
f: Generator[((int, None))]
g: Generator[int, None, None, None]
h: AsyncGenerator[(int), (None)]
