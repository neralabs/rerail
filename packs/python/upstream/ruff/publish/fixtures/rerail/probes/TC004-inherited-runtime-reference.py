from typing import TYPE_CHECKING
import foo
foo.x()
if TYPE_CHECKING:
    import foo
