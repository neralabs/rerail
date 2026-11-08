"""Retain raw source for parsed test arguments; never compute canonical expression text."""
import ast
from dataclasses import dataclass


@dataclass
class Origin:
    tree: ast.AST
    source: str
    snapshot: str


class Sources:
    """External assertion transport for unchanged ASTs created by ast.parse.

    This registry belongs to the test host. The native rule receives raw source
    and uses its own parser and canonical printer. Constructed or changed trees
    need a structured-value adapter; they must never use a guessed source string.
    """
    def __init__(self, parse):
        self.original_parse=parse
        self.origins={}

    def parse(self, source, *args, **kwargs):
        tree=self.original_parse(source,*args,**kwargs)
        if isinstance(source,str) and isinstance(tree,ast.AST):
            origin=Origin(tree,source,ast.dump(tree,include_attributes=True))
            for node in ast.walk(tree):self.origins[node]=origin
        return tree

    def segment(self,node):
        if node not in self.origins:
            raise NotImplementedError('A constructed AST needs structured-value transport; no parsed source was recorded')
        origin=self.origins[node]
        if ast.dump(origin.tree,include_attributes=True)!=origin.snapshot:
            raise NotImplementedError('A changed AST needs structured-value transport; its parsed source is stale')
        value=ast.get_source_segment(origin.source,node)
        if value is None:
            raise NotImplementedError('This AST value has no source span')
        return value
