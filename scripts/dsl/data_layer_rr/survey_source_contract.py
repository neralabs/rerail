"""Compare native survey producer fields with the pinned Python builder, offline only.

This is a producer-field contract, not full dead-code migration acceptance.
The native command reads a prior recording and never imports the source scanner.
"""
import argparse
import ast
import importlib
import json
from pathlib import Path
import sys
import tempfile

from migration import PIN, ROOT, git, sha, write_json


def deep_inheritance(depth):
    return 'class B0:\n def method(self): pass\n def __init__(self): pass\n'+''.join(
        f'class B{i}(B{i-1}): pass\n' for i in range(1,depth+1))


CASES = [
    ('empty', {'empty.py': ''}),
    ('definitions', {'impl/a.py': 'def one(): pass\nasync def two(): pass\nclass C:\n def method(self): pass\n async def amethod(self): pass\n'}),
    ('nested-functions', {'a.py': 'def outer():\n def inner(): pass\n class Hidden:\n  def omitted(self): pass\n return inner\n'}),
    ('nested-classes', {'a.py': 'class Outer:\n class Inner:\n  def method(self): pass\n def after(self): pass\n'}),
    ('conditional-definitions', {'a.py': 'if flag:\n def f(): pass\nelse:\n def f(): pass\nclass C:\n if flag:\n  def method(self): pass\n'}),
    ('decorated-order', {'a.py': '@decorate\ndef f(): pass\n@other\nclass C:\n @property\n def field(self): pass\n'}),
    ('unicode', {'a.py': 'class K:\n def ﬁeld(self): pass\ndef K(): pass\n'}),
    ('package-paths', {'pkg/__init__.py': 'class C: pass\n', 'pkg/child.py': 'def f(): pass\n', '__init__.py': 'def root(): pass\n'}),
    ('plain-imports', {'a.py': 'import x.y, first as second\nfrom lib import C, value as alias\nfrom lib import *\n'}),
    ('import-overwrite', {'a.py': 'from first import x\ndef f():\n from second import x\n import nested.module\nclass C:\n import final as nested\n'}),
    ('relative-imports', {'pkg/sub/a.py': 'from . import one\nfrom ..base import Two as Alias\nfrom ... import three\nfrom ....b import Four\n', 'pkg/sub/__init__.py': 'from . import one\nfrom ..base import Two\n', 'a.py': 'from . import skip\nfrom ..base import X\n'}),
    ('duplicate-aliases', {'a.py': 'from first import a as x, b as x\nimport a as y, b as y\nfrom third import x\n'}),
    ('future-imports', {'a.py': 'from __future__ import annotations\ndef f(): pass\n'}),
    ('signature-kinds', {'a.py': 'def f(pos: int, /, x: list[int], y=1, *args: str, z: A | None = None, **kwargs: object) -> tuple[int, str]: pass\nclass C:\n def m(self, cls: int, x: str, *, self: int = 0, cls: bool = True): pass\n'}),
    ('signature-empty', {'a.py': 'def f(): pass\nclass C:\n def m(self, /, *args, **kwargs): pass\n'}),
    ('signature-canonical', {'a.py': 'async def f(x: (((A| B))), y: "A") -> dict [str,list[tuple[A,B]]]: pass\n'}),
    ('module-refs', {'a.py': 'import ignored\nvalue = target\ndef omitted(x=NoDefault) -> NoReturn:\n return NoBody\nclass C(NoBase):\n field: FieldType = DefaultValue\n @decorate(DecoratorArg)\n def m(self, x: NoParam = MethodDefault, *, y=KeywordDefault) -> NoResult:\n  return NoMethodBody\n'}),
    ('conditional-module-refs', {'a.py': 'if flag:\n def f(x=Default):\n  return IncludedBody\nclass C:\n if flag:\n  def m(self, x=OtherDefault):\n   return IncludedMethodBody\n class Nested(Base):\n  def n(self): return NestedBody\n'}),
    ('module-ref-bindings', {'a.py': 'values = [x for x in source if pred(x)]\na: Alias = factory()\nclass C:\n store = target\n def m(self, /, x=PosDefault, *, y=KwDefault): pass\n'}),
    ('bases-and-fields', {'a.py': 'class C(pkg.ABC, Generic[A], metaclass=Meta):\n first: list[A]\n second: "B" = value\n first: C | None = None\n other.member: int = 0\n if flag:\n  hidden: int\n'}),
    ('decorator-spelling', {'a.py': '@pkg.decorate(1)\n@((factory()))\nclass C:\n @pkg.abstractmethod\n @wrap(value="(")\n def f(self): pass\n @property\n def g(self): pass\n'}),
    ('complex-decorators-and-bases', {'a.py': '@(lambda f: f)\n@(registry["key"])\nclass C(*bases, **keywords):\n @((make_decorator())(1))\n async def method(self): pass\nclass D(factory("ABC"), x if flag else y): pass\n'}),
    ('multiple-definition-files', {'impl/a.py': '@first\nclass A(ABC):\n x: int\n def method(self, x: int): pass\n', 'impl/b.py': '@second(1)\nclass B(pkg.Base):\n y: str\n def method(self, y: str): pass\n'}),
    ('body-call-shapes', {'a.py': 'def f():\n direct()\n pkg.method()\n self.method()\n cls.method()\n super().method()\n self.field.method()\n cls.field.method()\n factory().method()\n pkg.field.method()\n'}),
    ('body-argument-duplicates', {'a.py': 'def f():\n register(callback, self.method, pkg.method, key=other, **options)\n register(*spread, **keywords)\n register((callback), x=(self.method), y=(pkg.method))\n'}),
    ('body-local-order', {'a.py': 'def f(param, /, *args, keyword, **kwargs):\n unknown\n param\n args\n keyword\n kwargs\n unknown = source\n unknown\n del later\n later\n bound = bound\n bound()\n'}),
    ('body-argument-before-traversal', {'a.py': 'def f():\n consume((name := value), name)\n ((callee := factory))(callee)\n consume(inner(name), name)\n'}),
    ('body-attributes', {'a.py': 'def f():\n pkg.method\n self.method\n cls.method\n self.field.method\n super().method\n pkg.method = target\n del self.field\n registry = {"x": self.method}\n'}),
    ('body-builtins', {'a.py': 'def f():\n len(values)\n register(len, self, cls, super, range, missing)\n len\n frozenset\n pow\n len.method\n'}),
    ('body-nested-definitions', {'a.py': 'def outer():\n @decorate(callback)\n def inner(x: Annotation = Default) -> Return:\n  consume(x, callback)\n  saved = value\n saved\n inner\n class Nested(Base):\n  field: Type = factory()\n  def method(self, param=Default):\n   consume(param)\n'}),
    ('body-imports', {'pkg/a.py': 'def f():\n imported\n from .first import thing as imported\n imported\n imported()\n import pkg.module as imported\n imported\n from other import last as imported\n def nested():\n  import nested.module\n module\n'}),
    ('body-import-duplicates', {'a.py': 'def f():\n import first as alias, second as alias\n from pkg import a as same, b as same\n import final as same\n from ignored import *\n'}),
    ('body-execution-hits', {'a.py': 'def f(execute):\n execute()\n obj.execute_async()\n execute_grammar_custom()\n obj.grammar_fetch_extra()\n raw_execute()\n fetchall\n fetchone()\n unrelated()\n'}),
    ('body-comprehension-lambda', {'a.py': 'def f():\n values = [transform(item) for item in source if predicate(item)]\n callback = lambda arg: consume(arg)\n mapping = {(key := source): target(key), other: value}\n consume(key)\n'}),
    ('body-except-and-patterns', {'a.py': 'def f():\n global external\n try:\n  action()\n except Error as error:\n  consume(error)\n match subject:\n  case pkg.C(value) if predicate(value):\n   consume(value)\n'}),
    ('body-conditional-visit-order', {'a.py': 'def f():\n result = consume(name) if (name := condition) else other(name)\n consume({(key := source): value, other: consume(key)})\n'}),
    ('body-relative-imports', {'__init__.py': 'def f():\n from . import skip\n from ..base import X\n', 'pkg/__init__.py': 'def f():\n from . import A\n from ..base import B\n'}),
    ('body-non-simple-execution', {'a.py': 'def f():\n factory().grammar_fetch_opaque()\n registry[key].execute_grammar_opaque()\n self.worker.grammar_fetch_private()\n factory().execute()\n'}),
    ('body-generator-arguments', {'a.py': 'def f():\n consume(transform(item) for item in source)\n consume(x for x in source if pred(x))\n consume(*(factory(x) for x in source), **keywords)\n'}),
    ('body-more-binding-forms', {'a.py': 'def f():\n for first, *rest in source:\n  consume(first, rest)\n with manager() as resource:\n  consume(resource)\n x += x\n x\n callback = lambda *, only=default: consume(only)\n'}),
    ('graph-duplicates', {'a.py': 'class Base:\n def old(self): pass\n def replace(self): pass\nclass Base:\n def new(self): pass\n def replace(self): pass\nclass C(Base): pass\n'}),
    ('graph-type-replacement', {'a.py': 'class Base: pass\ndef Base(): pass\nclass Local(Base): pass\n', 'other.py': 'class Remote(Base): pass\n'}),
    ('graph-path-collision', {'a.py': 'from first import x\nclass Base: pass\ndef early(): pass\n', 'a/__init__.py': 'from last import x\nclass Base: pass\ndef late(): pass\n'}),
    ('graph-reversed-path-collision', {'a/__init__.py': 'from last import x\nclass Base: pass\ndef late(): pass\n', 'a.py': 'from first import x\nclass Base: pass\ndef early(): pass\n'}),
    ('graph-ambiguous-global', {'a.py': 'class Base: pass\n', 'b.py': 'class Base: pass\n', 'entry.py': 'class C(Base): pass\n'}),
    ('graph-imported-bases', {'a.py': 'class Base: pass\ndef factory(): pass\n', 'entry.py': 'from a import Base as Alias, factory as Function\nclass C(Alias): pass\nclass D(Function): pass\n'}),
    ('graph-unresolved-import-base', {'a.py': 'class Base: pass\n', 'entry.py': 'from missing import Base\nclass Local: pass\nclass C(Base, Local): pass\n'}),
    ('graph-bases-no-reexports', {'a.py': 'class Base: pass\n', 'b.py': 'class Base: pass\n', 'bridge.py': 'from a import Base as Public\n', 'entry.py': 'from bridge import Public\nclass C(Public): pass\n'}),
    ('graph-generic-spelling', {'a.py': 'class Base: pass\nclass C(Base[int], pkg.Base[Other], factory(Base)): pass\n'}),
    ('graph-reexport-cycles', {'a.py': 'from b import Alias\nfrom c import concrete as Reexport\ndef direct(): pass\n', 'b.py': 'from a import Alias\n', 'c.py': 'from missing import concrete\ndef concrete(): pass\n'}),
    ('name-call-own-priority', {'a.py': 'def target(): pass\nclass C:\n def run(self): target()\n def target(self): pass\n'}),
    ('name-call-method-before-class', {'a.py': 'class target: pass\nclass One:\n def target(self): pass\nclass Two:\n def target(self): pass\ndef run(): target()\n'}),
    ('name-call-method-before-function', {'a.py': 'class One:\n def target(self): pass\nclass Two:\n def target(self): pass\ndef target(): pass\ndef run(): target()\n'}),
    ('name-call-imports-and-reexports', {'a.py': 'def target(): pass\nclass C: pass\n', 'bridge.py': 'from a import target as Alias\n', 'entry.py': 'from bridge import Alias\nfrom a import C\ndef run():\n Alias()\n C()\n missing()\n'}),
    ('name-call-local-plain-import', {'a.py': 'def target(): pass\n', 'entry.py': 'def run():\n import a.target as callback\n callback()\n'}),
    ('name-call-unresolved-import-blocks-local', {'a.py': 'def target(): pass\n', 'entry.py': 'from missing import callback\ndef run():\n import a.target as callback\n callback()\n'}),
    ('name-call-bound-parameter', {'a.py': 'def target(): pass\ndef len(): pass\ndef run(target):\n target()\n len([])\n'}),
    ('name-call-local-class', {'a.py': 'class Target: pass\ndef run(): Target()\n'}),
    ('name-call-local-before-import', {'a.py': 'def target(): pass\n', 'entry.py': 'def target(): pass\nfrom a import target\ndef run(): target()\n'}),
    ('name-call-hoisted-import', {'a.py': 'def first(): pass\n', 'b.py': 'def target(): pass\n', 'entry.py': 'def run():\n from a import first as callback\n callback()\ndef later():\n from b import target as callback\n'}),
    ('name-call-reexport-cycle', {'a.py': 'from b import Alias\n', 'b.py': 'from a import Alias\n', 'entry.py': 'from a import Alias\ndef run(): Alias()\n'}),
    ('walk-diamond', {'a.py': 'class A:\n def method(self): pass\nclass Left(A): pass\nclass Right(A):\n def method(self): pass\nclass Leaf(Left, Right):\n def run(self):\n  self.method()\n  super().method()\n'}),
    ('walk-reversed-diamond', {'a.py': 'class A:\n def method(self): pass\nclass Left(A):\n def method(self): pass\nclass Right(A):\n def method(self): pass\nclass Leaf(Right, Left): pass\n'}),
    ('walk-public-eligibility', {'a.py': 'class Base:\n def method(self): pass\n def alpha(self): pass\n def __init__(self): pass\nclass C(Base):\n @property\n def method(self): pass\n def zebra(self): pass\n def _private(self): pass\n def authorize_internal_access(self): pass\n'}),
    ('walk-dunder-order', {'a.py': 'class Base:\n def __post_init__(self): pass\n def __init__(self): pass\n def __exit__(self): pass\nclass C(Base):\n def __new__(cls): pass\n def __init__(self): pass\n'}),
    ('walk-cycle', {'a.py': 'class A(B):\n def method(self): pass\n def __init__(self): pass\nclass B(A):\n def other(self): pass\n def __init__(self): pass\n'}),
    ('walk-self-cycle', {'a.py': 'class C(C):\n def method(self): pass\n def __init__(self): pass\n'}),
    ('walk-function-base', {'a.py': 'class Factory:\n def method(self): pass\ndef Factory(): pass\n', 'entry.py': 'from a import Factory as Alias\nclass C(Alias): pass\n'}),
    ('walk-deeper-than-64', {'a.py': deep_inheritance(70)}),
    ('member-self-and-super', {'a.py': 'class Base:\n def _private(self): pass\n @property\n def prop(self): pass\nclass C(Base):\n def _private(self): pass\n def run(self):\n  self._private()\n  self.prop\n  super()._private()\n  self.missing()\n'}),
    ('member-mixin-single', {'a.py': 'class Base:\n def run(self): self._hook()\nclass Mixin:\n def _hook(self): pass\nclass C(Base, Mixin): pass\n'}),
    ('member-cobase-order', {'a.py': 'class Base:\n def run(self): self._hook()\nclass First:\n def _hook(self): pass\nclass Second:\n def _hook(self): pass\nclass C(Base, First, Second): pass\n'}),
    ('member-module-reexport', {'impl.py': 'def target(): pass\n', 'bridge.py': 'from impl import target as Alias\n', 'entry.py': 'import impl as module\nimport bridge\ndef run():\n module.target()\n bridge.Alias()\n module.missing()\n'}),
    ('member-imported-class', {'impl.py': 'class Base:\n def method(self): pass\nclass C(Base): pass\n', 'bridge.py': 'from impl import C as Public\n', 'entry.py': 'from bridge import Public as Alias\ndef run(obj: Alias):\n Alias.method()\n obj.method()\n'}),
    ('member-local-types', {'a.py': 'class C:\n def method(self): pass\ndef run(first: C | None, second: C[int], quoted: "C", qualified: pkg.C):\n first.method()\n second.method()\n quoted.method()\n qualified.method()\n C.method()\n'}),
    ('member-import-module-blocks-parameter', {'a.py': 'import missing as obj\nclass C:\n def method(self): pass\ndef run(obj: C): obj.method()\n'}),
    ('member-import-class-blocks-local', {'impl.py': 'class Imported: pass\n', 'entry.py': 'class Alias:\n def method(self): pass\nfrom impl import Imported as Alias\ndef run(): Alias.method()\n'}),
    ('member-local-class-blocks-parameter', {'a.py': 'class obj: pass\nclass Other:\n def method(self): pass\ndef run(obj: Other): obj.method()\n'}),
    ('member-constructor-and-class-annotations', {'a.py': 'class Worker:\n def method(self): pass\nclass Other:\n def method(self): pass\nclass Base:\n worker: Other\nclass C(Base):\n worker: Other\n def __init__(self, worker: Worker): pass\n def run(self):\n  self.worker.method()\n  self._worker.method()\n'}),
    ('member-annotation-fallbacks', {'a.py': 'class Worker:\n def method(self): pass\nclass Other:\n def method(self): pass\nclass Base:\n worker: Worker\nclass C(Base):\n _worker: Other\n def __init__(self, worker: Worker, _worker): pass\n def run(self):\n  self._worker.method()\n  self.worker.method()\n'}),
    ('member-last-class-first-parameter', {'a.py': 'class First:\n def method(self): pass\nclass Second:\n def method(self): pass\nclass C:\n worker: First\n worker: Second\n def run(self): self.worker.method()\ndef run(obj: First, obj: Second): obj.method()\n'}),
    ('member-local-import-after-module-replacement', {'a.py': 'def run(obj):\n from impl import C as obj\n obj.method()\n', 'a/__init__.py': '', 'impl.py': 'class C:\n def method(self): pass\n'}),
    ('member-inherited-hint-caller-module', {'base.py': 'class Worker:\n def method(self): pass\nclass Base:\n worker: Worker\n', 'entry.py': 'from base import Base\nclass Worker:\n def method(self): pass\nclass C(Base):\n def run(self): self.worker.method()\n'}),
    ('member-unannotated-exact-inherited-hint', {'a.py': 'class Worker:\n def method(self): pass\nclass Other:\n def method(self): pass\nclass Base:\n _worker: Other\nclass C(Base):\n def __init__(self, worker: Worker, _worker): pass\n def run(self): self._worker.method()\n'}),
    ('member-imported-nonclass-stale-method', {'a.py': 'class Factory:\n def method(self): pass\ndef Factory(): pass\n', 'entry.py': 'from a import Factory as obj\ndef run(): obj.method()\n'}),
    ('member-missing-param-method-local-import', {'a.py': 'def run(obj: Other):\n from impl import C as obj\n obj.method()\n', 'a/__init__.py': 'class Other: pass\n', 'impl.py': 'class C:\n def method(self): pass\n'}),
    ('member-first-unannotated-duplicate-parameter', {'a.py': 'class C:\n def method(self): pass\ndef run(obj, obj: C): obj.method()\n'}),
    ('edges-function-class-dedup', {'a.py': 'class C: pass\ndef helper(): pass\ndef run():\n helper()\n helper()\n consume(helper, helper)\n C()\n consume(C)\n'}),
    ('edges-legacy-sync-async', {'a.py': 'class C:\n @legacy_sync\n def run(self): pass\n def run_lazy(self): pass\n @pkg.direct_legacy_module_async\n async def fetch_async(self): pass\n def fetch_lazy(self): pass\n @legacy_async\n def other(self): pass\n def other_lazy(self): pass\n'}),
    ('edges-legacy-sync-wins', {'a.py': 'class C:\n @legacy_sync\n @legacy_async\n def run_async(self): pass\n def run_async_lazy(self): pass\n def run_lazy(self): pass\n'}),
    ('edges-legacy-inherited-first', {'a.py': 'class Far:\n def run_lazy(self): pass\nclass Left(Far): pass\nclass Right:\n def run_lazy(self): pass\nclass C(Left, Right):\n @legacy_module_sync\n def run(self): pass\n'}),
    ('edges-legacy-property-missing-free', {'a.py': '@legacy_sync\ndef run(): pass\ndef run_lazy(): pass\nclass C:\n @legacy_sync\n def missing(self): pass\n @legacy_sync\n def value(self): pass\n @property\n def value_lazy(self): pass\n'}),
    ('exports-implicit-imports-and-locals', {'impl.py': 'def target(): pass\nclass C:\n def method(self): pass\n', 'data_layer/api.py': 'from impl import target as Alias, C\nimport impl as module\ndef local(): pass\ndef _private(): pass\n'}),
    ('exports-explicit-duplicates', {'impl.py': 'class C:\n def method(self): pass\n', 'data_layer/__init__.py': 'from impl import C\n__all__ = ["C", "missing", "_private", "C", 1, b"C"]\n'}),
    ('exports-first-all-only', {'data_layer/a.py': 'def First(): pass\ndef Second(): pass\n__all__ = ["Second"]\n__all__ = ["First"]\n'}),
    ('exports-unsupported-all-blocks', {'data_layer/a.py': 'def C(): pass\n__all__ = frozenset({"C"})\n__all__ = ["C"]\n'}),
    ('exports-empty-all-blocks', {'data_layer/a.py': 'def C(): pass\n__all__ = []\n'}),
    ('exports-annotated-all-ignored', {'data_layer/a.py': 'def C(): pass\n__all__: list[str] = []\n'}),
    ('exports-chained-all-tuple', {'data_layer/a.py': 'def C(): pass\nother = __all__ = ("C", ("C"), None)\n'}),
    ('exports-import-over-definition', {'impl.py': 'def target(): pass\n', 'data_layer/a.py': 'from impl import target as C\nclass C: pass\n'}),
    ('exports-top-level-only', {'impl.py': 'def target(): pass\n', 'data_layer/a.py': 'if flag:\n from impl import target as conditional\n def hidden(): pass\ndef outer():\n from impl import target as nested\n if flag:\n  def inner(): pass\n'}),
    ('exports-direct-child-files-only', {'data_layer/a.py': 'def Direct(): pass\n', 'data_layer/sub/a.py': 'def Nested(): pass\n', 'other.py': 'def Other(): pass\n'}),
    ('exports-sorted-paths', {'data_layer/z.py': 'def Zed(): pass\n', 'data_layer/a.py': 'def Alpha(): pass\n'}),
    ('exports-reexports-cycle-and-missing', {'impl.py': 'class C: pass\n', 'bridge.py': 'from impl import C as Public\nfrom data_layer.api import Loop\n', 'data_layer/api.py': 'from bridge import Public, Loop\nfrom missing import Absent\n'}),
    ('exports-public-eligibility', {'data_layer/api.py': 'class Base:\n def inherited(self): pass\nclass C(Base):\n def zed(self): pass\n @property\n def inherited(self): pass\n def _private(self): pass\n def __init__(self): pass\n def authorize_internal_access(self): pass\n__all__ = ["C"]\n'}),
    ('exports-unicode-and-decorated', {'data_layer/api.py': '@decorator\nclass K:\n def method(self): pass\n@decorator\ndef ﬁeld(): pass\n'}),
]


# These assertions guard the fixtures' intended positive branches. In particular,
# defining a function with a class's name replaces its symbol and can accidentally
# turn a method-before-class fixture into a different test while parity stays green.
TARGET_CONTRACTS = [
    ('name-call-method-before-class','a::run','target','a::One.target','method',1),
    ('name-call-method-before-function','a::run','target','a::One.target','method',1),
    ('name-call-own-priority','a::C.run','target','a::C.target','method',1),
    ('name-call-local-class','a::run','Target','a::Target','class',1),
    ('name-call-hoisted-import','entry::run','callback','b::target','function',1),
    ('name-call-unresolved-import-blocks-local','entry::run','callback','','',0),
]

MEMBER_CONTRACTS = [
    ('member-mixin-single','a::Base.run','self_attr','self','_hook','a::Mixin._hook','method',1),
    ('member-cobase-order','a::Base.run','self_attr','self','_hook','a::First._hook','method',1),
    ('member-import-module-blocks-parameter','a::run','module_attr','obj','method','','',0),
    ('member-import-class-blocks-local','entry::run','module_attr','Alias','method','','',0),
    ('member-local-class-blocks-parameter','a::run','module_attr','obj','method','','',0),
    ('member-inherited-hint-caller-module','entry::C.run','chained_self_attr','worker','method','entry::Worker.method','method',1),
    ('member-unannotated-exact-inherited-hint','a::C.run','chained_self_attr','_worker','method','a::Other.method','method',1),
    ('member-imported-nonclass-stale-method','entry::run','module_attr','obj','method','a::Factory.method','method',1),
    ('member-missing-param-method-local-import','a::run','module_attr','obj','method','impl::C.method','method',1),
    ('member-first-unannotated-duplicate-parameter','a::run','module_attr','obj','method','','',0),
]

EDGE_CONTRACTS = [
    ('edges-function-class-dedup','graph_call','a::run','a::helper'),
    ('edges-function-class-dedup','graph_class_use','a::run','a::C'),
    ('edges-function-class-dedup','graph_caller','a::helper','a::run'),
    ('edges-legacy-sync-async','graph_call','a::C.run','a::C.run_lazy'),
    ('edges-legacy-sync-async','graph_call','a::C.fetch_async','a::C.fetch_lazy'),
    ('edges-legacy-sync-async','graph_call','a::C.other','a::C.other_lazy'),
    ('edges-legacy-sync-wins','graph_call','a::C.run_async','a::C.run_async_lazy'),
    ('edges-legacy-inherited-first','graph_call','a::C.run','a::Right.run_lazy'),
    ('edges-legacy-property-missing-free','graph_call','a::C.value','a::C.value_lazy'),
]

EXPORT_CONTRACTS = [
    ('exports-implicit-imports-and-locals', 'data_layer.api', 'Alias', 'impl::target'),
    ('exports-import-over-definition', 'data_layer.a', 'C', 'impl::target'),
    ('exports-annotated-all-ignored', 'data_layer.a', 'C', 'data_layer.a::C'),
    ('exports-public-eligibility', 'data_layer.api', 'C', 'data_layer.api::C'),
    ('exports-reexports-cycle-and-missing', 'data_layer.api', 'Public', 'impl::C'),
]


def check_target_contracts(cases):
    by_id={case['id']:case for case in cases}
    for label,owner,name,target,kind,has in TARGET_CONTRACTS:
        rows=by_id[label]['expected']
        if not any(row['kind']=='graph_name_call' and
                   (row['owner'],row['name'],row['target'],row['target_kind'],row['has_target']) == (owner,name,target,kind,has)
                   for row in rows):
            raise ValueError(f'The source fixture no longer covers {label}: {owner}/{name}')
    for label,owner,ref_kind,base,attr,target,kind,has in MEMBER_CONTRACTS:
        expected=dict(owner=owner,ref_kind=ref_kind,base=base,attr=attr,target=target,target_kind=kind,has_target=has)
        if not any(row['kind']=='graph_ref' and all(row.get(k)==v for k,v in expected.items())
                   for row in by_id[label]['expected']):
            raise ValueError(f'The source fixture no longer covers {label}: {expected}')
    for label,kind,owner,sid in EDGE_CONTRACTS:
        if dict(kind=kind,owner=owner,sid=sid) not in by_id[label]['expected']:
            raise ValueError(f'The source fixture no longer covers {label}: {owner} -> {sid}')
    for label,module,name,sid in EXPORT_CONTRACTS:
        if not any(row['kind']=='graph_export' and (row['module'],row['name'],row['sid'])==(module,name,sid)
                   for row in by_id[label]['expected']):
            raise ValueError(f'The source fixture no longer covers {label}: {module}/{name}')


def source_rows(source, files, scan_paths=None):
    sys.path[:0] = [str(source), str(source / 'scripts')]
    model = importlib.import_module('lint_lib.symbol_graph')
    rows, scans = [], []
    for path in files if scan_paths is None else scan_paths:
        text = files[path]
        module, info, symbols = model._scan_symbol_tree(Path('/survey'), Path('/survey') / path, ast.parse(text))
        scans.append((module,info,symbols))
        rows.append(dict(kind='module', path=path, module=module))
        for i, s in enumerate(symbols):
            rows.append(dict(kind='symbol', path=path, module=module, ordinal=i, sid=s.symbol_id,
                             fq=s.fqname, name=s.name, qual=s.qualname, symbol_kind=s.kind,
                             line=s.line, owner=s.class_name or '', async_flag=int(s.is_async), abstract_flag=int(s.is_abstract)))
            for kind, values in [('decorator',s.decorators),('class_decorator',s.class_decorators),('base',s.bases)]:
                rows.extend(dict(kind=kind,path=path,symbol_ordinal=i,ordinal=j,text=text) for j,text in enumerate(values))
            rows.extend(dict(kind='class_attribute',path=path,symbol_ordinal=i,ordinal=j,name=name,text=text)
                        for j,(name,text) in enumerate(s.class_attr_annotations))
            if s.kind != 'class':
                rows.append(dict(kind='return', path=path, symbol_ordinal=i, has_annotation=int(s.return_annotation is not None), text=s.return_annotation or ''))
                for j, (name, ann) in enumerate(zip(s.param_names, s.param_annotations)):
                    rows.append(dict(kind='parameter', path=path, symbol_ordinal=i, ordinal=j, name=name,
                                     has_annotation=int(ann is not None), text=ann or ''))
                rows.extend(dict(kind='raw_ref', path=path, symbol_ordinal=i, ordinal=j,
                                 ref_kind=ref.kind, base=ref.base or '', has_base=int(ref.base is not None), attr=ref.attr)
                            for j,ref in enumerate(s.raw_refs))
                rows.extend(dict(kind='execution_hit',path=path,symbol_ordinal=i,name=name) for name in s.execution_hits)
                rows.extend(dict(kind='local_import',path=path,symbol_ordinal=i,name=name,target=target)
                            for name,target in s.local_imports.items())
        for name, ids in info.defined_names.items():
            for i, sid in enumerate(ids):
                rows.append(dict(kind='defined', path=path, module=module, name=name, sid=sid, ordinal=i))
        for kind, values in [('imported', info.imported_symbols), ('import_module', info.imported_modules)]:
            rows.extend(dict(kind=kind, path=path, module=module, name=k, target=v) for k, v in values.items())
        rows.extend(dict(kind='module_ref', path=path, module=module, name=name) for name in info.module_level_names)
    graph=model.SymbolGraph(Path('/survey'))
    graph._add_scanned_modules(scans)
    rows.extend(dict(kind='graph_symbol',ordinal=i,sid=sid,path=s.path,line=s.line,symbol_kind=s.kind)
                for i,(sid,s) in enumerate(graph.symbols.items()))
    rows.extend(dict(kind='graph_module',ordinal=i,module=name,path=info.path.relative_to('/survey').as_posix())
                for i,(name,info) in enumerate(graph.modules.items()))
    rows.extend(dict(kind='graph_fqname',name=name,sid=sid) for name,sid in graph.symbol_by_fqname.items())
    rows.extend(dict(kind='graph_class_name',name=name,ordinal=i,sid=sid)
                for name,ids in graph.class_ids_by_name.items() for i,sid in enumerate(ids))
    rows.extend(dict(kind='graph_method',owner=owner,name=name,ordinal=i,sid=sid)
                for owner,methods in graph.class_methods.items() for i,(name,sid) in enumerate(methods.items()))
    rows.extend(dict(kind='graph_base',owner=owner,ordinal=i,sid=sid)
                for owner,bases in graph.class_bases.items() for i,sid in enumerate(bases))
    rows.extend(dict(kind='graph_subclass',owner=owner,sid=sid) for owner,subs in graph.class_subclasses.items() for sid in subs)
    for owner in sorted(set(graph.class_bases)|set(graph.class_methods)):
        rows.append(dict(kind='graph_accessor',owner=owner))
        for kind,values in [('graph_walk',graph.walk_base_classes(owner)),
                            ('graph_public',graph.public_methods_of_class(owner)),
                            ('graph_dunder',graph._class_dunder_methods(owner))]:
            rows.extend(dict(kind=kind,owner=owner,ordinal=i,sid=sid) for i,sid in enumerate(values))
    queries=set(graph.symbol_by_fqname)
    for name,info in graph.modules.items():
        queries.update(info.imported_symbols.values())
        queries.update(name+'.'+local for local in info.imported_symbols)
    rows.extend(dict(kind='graph_resolution',name=name,sid=sid) for name in sorted(queries)
                if (sid:=graph._resolve_fqname_through_reexports(name)) is not None)
    for sid,symbol in graph.symbols.items():
        if not symbol.is_function:continue
        for i,ref in enumerate(symbol.raw_refs):
            targets=graph._resolve_ref(symbol,ref)
            assert len(targets)<=1
            target=targets[0] if targets else None
            rows.append(dict(kind='graph_ref',owner=sid,ordinal=i,ref_kind=ref.kind,base=ref.base or '',attr=ref.attr,
                             has_target=int(target is not None),target=target or '',
                             target_kind=graph.symbols[target].kind if target is not None else ''))
            if ref.kind!='name_call':continue
            target=graph._resolve_name_call(symbol,graph.modules[symbol.module],ref.attr)
            rows.append(dict(kind='graph_name_call',owner=sid,ordinal=i,name=ref.attr,
                             has_target=int(target is not None),target=target or '',
                             target_kind=graph.symbols[target].kind if target is not None else ''))
    graph._resolve_edges()
    graph._add_legacy_decorator_edges()
    for sid,symbol in graph.symbols.items():
        rows.extend(dict(kind='graph_call',owner=sid,sid=target) for target in symbol.calls)
        rows.extend(dict(kind='graph_class_use',owner=sid,sid=target) for target in symbol.class_uses)
    rows.extend(dict(kind='graph_caller',owner=target,sid=caller)
                for target,callers in graph.callers.items() for caller in callers)
    with tempfile.TemporaryDirectory(prefix='rerail-offline-source-exports-') as temporary:
        root=Path(temporary)
        for path,text in files.items():
            target=root/path
            target.parent.mkdir(parents=True,exist_ok=True)
            target.write_text(text,encoding='utf-8')
        graph.root=root
        graph._collect_data_layer_roots()
    rows.extend(dict(kind='graph_export',ordinal=i,module=r.export_module,name=r.export_name,sid=r.symbol_id)
                for i,r in enumerate(graph.exported_roots))
    rows.extend(dict(kind='graph_exported_root',sid=sid) for sid in graph.exported_root_ids)
    rows.extend(dict(kind='graph_exported_class',sid=sid) for sid in graph.exported_class_ids)
    return ordered(rows)


def ordered(rows):
    return sorted(rows, key=lambda r: json.dumps(r, sort_keys=True, ensure_ascii=False))


def native_rows(diagnostics):
    """Render ordered dictionary DTOs from native order keys, without graph logic.

    Native relations keep first file/local keys instead of computing a dense
    global rank with an all-pairs count. Only this test DTO needs list indices.
    """
    rows=[dict(r['parameters']) for r in diagnostics]
    for kind in ['graph_symbol','graph_module']:
        selected=[r for r in rows if r['kind']==kind]
        selected.sort(key=lambda r:(r['order_file'],r['order_local'] if kind=='graph_symbol' else 0))
        for i,row in enumerate(selected):
            row['ordinal']=i
            del row['order_file']
            if kind=='graph_symbol':del row['order_local']
    return ordered(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    parser.add_argument('--source', type=Path)
    parser.add_argument('--engine', type=Path)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--rule', type=Path, default=ROOT/'test/fixtures/rr/survey-source.rr')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    args = parser.parse_args()
    if args.parts < 1 or not 0 <= args.part < args.parts:
        parser.error('Require parts >= 1 and 0 <= part < parts')
    evidence = dict(format='rerail.survey-source-inventory/v1', sourcePin=PIN,
                    fullMigrationAcceptance=False, cases=[])
    if args.command == 'source':
        if not args.source or git(args.source, 'rev-parse', 'HEAD') != PIN:
            parser.error('The pinned source checkout is required')
        evidence['sourceSha256'] = sha((args.source/'scripts/lint_lib/symbol_graph.py').read_bytes())
        for label, files in CASES:
            evidence['cases'].append(dict(id=label, files=[dict(path=p, source=t) for p,t in files.items()],
                                          expected=source_rows(args.source, files)))
        check_target_contracts(evidence['cases'])
    else:
        from replay import execute_native
        baseline = json.loads(args.baseline.read_text())
        required = [(label, [dict(path=p, source=t) for p,t in files.items()]) for label,files in CASES]
        recorded = [(case['id'], case['files']) for case in baseline['cases']]
        if baseline['sourcePin'] != PIN or baseline['format'] != evidence['format']:
            parser.error('The source recording identity differs')
        if recorded != required or any(not case['expected'] for case in baseline['cases']):
            parser.error('The complete source fixture recording is required')
        check_target_contracts(baseline['cases'])
        evidence['caseSelection']=dict(part=args.part,parts=args.parts,total=len(baseline['cases']))
        evidence['baselineSha256'] = sha(args.baseline.read_bytes())
        evidence['engineSha256'] = sha(args.engine.read_bytes())
        evidence.update(complete=False,passed=False)
        write_json(args.output,evidence)
        for case in baseline['cases'][args.part::args.parts]:
            native = execute_native(args.engine, args.rule, case['files'], {})
            rows = native_rows(native['diagnostics'])
            evidence['cases'].append(dict(id=case['id'], expected=case['expected'], actual=rows,
                                          passed=rows == case['expected'], programInputs=native['programInputs']))
            write_json(args.output,evidence)
    evidence['complete']=True
    evidence['passed'] = all(c.get('passed', True) for c in evidence['cases'])
    write_json(args.output, evidence)
    print(json.dumps(dict(cases=len(evidence['cases']), passed=evidence['passed'],
                          failures=[c['id'] for c in evidence['cases'] if not c.get('passed', True)])))
    return 0 if evidence['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
