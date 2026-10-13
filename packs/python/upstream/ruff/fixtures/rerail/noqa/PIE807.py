from dataclasses import dataclass, field
@dataclass
class C:
    items: list = field(default_factory=lambda: [])

from dataclasses import dataclass, field
@dataclass
class C:
    items: list = field(default_factory=lambda: [])  # noqa

from dataclasses import dataclass, field
@dataclass
class C:
    items: list = field(default_factory=lambda: [])  # noqa: PIE807

from dataclasses import dataclass, field
@dataclass
class C:
    items: list = field(default_factory=lambda: [])  # NOQA: PIE807

from dataclasses import dataclass, field
@dataclass
class C:
    items: list = field(default_factory=lambda: [])  # noqa: E501
