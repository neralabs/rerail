# SPDX-License-Identifier: Apache-2.0
"""Canonical structural columns; no linter predicates or project-specific names."""
import ast

STRUCTURE_FIELDS = (
    'ast_kind','ast_name','ast_alias','ast_module','ast_level','ast_value','ast_value_kind',
    'ast_preorder','ast_breadth_order','ast_depth','ast_parent','ast_parent_kind',
    'ast_field','ast_index','ast_end','ast_function','ast_class','ast_op','ast_op_count',
    'ast_start','ast_stop','ast_line','ast_context','ast_name_repr','ast_value_repr','ast_annotation','ast_annotation_repr',
)
TEXT_FIELDS = ('source_lines','source_line','previous_line')


def structure_index(tree, offsets, source_size, breadth=True, annotations=True):
    rows={}; nodes=[]; depths=[]
    def visit(node,parent=None,field='',index=0,depth=0,fn=None,cls=None):
        ordinal=len(nodes);nodes.append(node);depths.append(depth)
        start=offsets[node.lineno-1]+node.col_offset if hasattr(node,'lineno') else 0
        stop=offsets[node.end_lineno-1]+node.end_col_offset if getattr(node,'end_lineno',None) else source_size if isinstance(node,ast.Module) else 0
        row=dict(ast_kind=type(node).__name__,ast_name=next((getattr(node,key) for key in ('name','id','arg','attr') if isinstance(getattr(node,key,None),str)),''),
          ast_alias=getattr(node,'asname',None) or '',ast_module=getattr(node,'module',None) or '',ast_level=str(node.level) if isinstance(node,ast.ImportFrom) else '',
          ast_preorder=str(ordinal),ast_depth=str(depth),ast_parent='' if parent is None else str(parent),ast_parent_kind='' if parent is None else type(nodes[parent]).__name__,
          ast_field=field,ast_index=str(index),ast_function='' if fn is None else str(fn),ast_class='' if cls is None else str(cls),
          ast_start=str(start),ast_stop=str(stop),ast_line=str(getattr(node,'lineno',1)),ast_context=type(node.ctx).__name__ if hasattr(node,'ctx') else '')
        operators=list(getattr(node,'ops',())) or ([node.op] if hasattr(node,'op') else [])
        row.update(ast_op='|'.join(type(op).__name__ for op in operators),ast_op_count=str(len(operators)))
        if isinstance(node,ast.Constant):
            value=node.value
            tag='none' if value is None else 'ellipsis' if value is Ellipsis else type(value).__name__
            row.update(ast_value_kind=tag,ast_value=value if isinstance(value,str) else value.hex() if isinstance(value,bytes) else str(value))
        else:row.update(ast_value_kind='',ast_value='')
        row['ast_name_repr']=repr(row['ast_name'])
        row['ast_value_repr']=repr(node.value) if isinstance(node,ast.Constant) else ''
        row['ast_annotation']=ast.unparse(node) if annotations and field in {'annotation','returns'} else ''
        row['ast_annotation_repr']=repr(row['ast_annotation'])
        rows[id(node)]=row
        next_fn=ordinal if isinstance(node,(ast.FunctionDef,ast.AsyncFunctionDef)) else fn
        next_cls=ordinal if isinstance(node,ast.ClassDef) else cls
        for name,value in ast.iter_fields(node):
            children=enumerate(value) if isinstance(value,list) else [(0,value)]
            for child_index,child in children:
                if isinstance(child,ast.AST):visit(child,ordinal,name,child_index,depth+1,next_fn,next_cls)
        row['ast_end']=str(len(nodes)-1)
    visit(tree)
    if breadth:
        for rank,(_,node) in enumerate(sorted(enumerate(nodes),key=lambda item:(depths[item[0]],item[0]))):
            rows[id(node)]['ast_breadth_order']=str(rank)
    return rows,nodes


def annotation_names(node):
    if isinstance(node,ast.Constant) and isinstance(node.value,str):
        try:return annotation_names(ast.parse(node.value,mode='eval').body)
        except SyntaxError:return [node.value]
    return sorted({child.id if isinstance(child,ast.Name) else child.attr for child in ast.walk(node) if isinstance(child,(ast.Name,ast.Attribute))})
