"""Passive field declarations from pinned source. Source methods are not copied."""
# Source SHA256: f4b8662365cde5364284bae71fa6821b3db5a483cfe7a6f3f3041428e6e7d13a
from typing import NewType
from pydantic import BaseModel, ConfigDict

ServerPlanFactPath = NewType('ServerPlanFactPath', str)

ServerPlanName = NewType('ServerPlanName', str)

ServerPlanQualname = NewType('ServerPlanQualname', str)

ServerPlanHitKind = NewType('ServerPlanHitKind', str)

ServerPlanMessage = NewType('ServerPlanMessage', str)

class ServerPlanClassFacts(BaseModel):
    """Class-level facts needed by ServerPlan architectural linters."""
    model_config = ConfigDict(frozen=True)
    name: ServerPlanName
    qualname: ServerPlanQualname
    line: int
    bases: tuple[str, ...] = ()
    decorators: tuple[str, ...] = ()
    is_step_subclass: bool = False

class ServerPlanFunctionFacts(BaseModel):
    """Function-level facts needed by ServerPlan architectural linters."""
    model_config = ConfigDict(frozen=True)
    name: ServerPlanName
    qualname: ServerPlanQualname
    line: int
    class_qualname: ServerPlanQualname | None = None
    is_async: bool = False
    decorators: tuple[str, ...] = ()
    return_annotation: str | None = None
    parameter_names: tuple[ServerPlanName, ...] = ()
    call_names: tuple[ServerPlanName, ...] = ()
    attr_refs: tuple[ServerPlanName, ...] = ()
    name_refs: tuple[ServerPlanName, ...] = ()
    assigned_names: tuple[ServerPlanName, ...] = ()
    string_constants: tuple[str, ...] = ()

class ServerPlanCompileMethodFacts(BaseModel):
    """Compile method attached to a ServerPlanStep subclass."""
    model_config = ConfigDict(frozen=True)
    class_name: ServerPlanName
    class_qualname: ServerPlanQualname
    function_qualname: ServerPlanQualname
    line: int

class GrammarSurfaceHit(BaseModel):
    """Syntactic grammar-surface finding projected from one file."""
    model_config = ConfigDict(frozen=True)
    kind: ServerPlanHitKind
    line: int
    symbol: ServerPlanName
    message: ServerPlanMessage

class ServerPlanFileFacts(BaseModel):
    """Per-file facts shared by ServerPlan linters."""
    model_config = ConfigDict(frozen=True)
    path: ServerPlanFactPath
    classes: tuple[ServerPlanClassFacts, ...] = ()
    functions: tuple[ServerPlanFunctionFacts, ...] = ()
    compile_methods: tuple[ServerPlanCompileMethodFacts, ...] = ()
    grammar_surface_hits: tuple[GrammarSurfaceHit, ...] = ()
    syntax_error: str | None = None

class ProjectServerPlanFacts(BaseModel):
    """Project-level assembly of per-file ServerPlan facts."""
    model_config = ConfigDict(frozen=True)
    files: tuple[ServerPlanFileFacts, ...]
