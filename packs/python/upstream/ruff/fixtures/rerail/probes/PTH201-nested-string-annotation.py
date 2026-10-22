from pathlib import Path
from typing import Annotated, List

a: """List['Path(".")']"""
b: Annotated["Path('')", "Path('.')"]
c: "((Path))('.')"
