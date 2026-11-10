"""Preserve a JSON tree's members, types and positions without validating a model."""
from fact_transport import encode_bundles, members


def encode_models(bundles):
    result=encode_bundles(bundles)
    for bundle in result:
        nodes=[]
        def visit(value,parent,key,ordinal):
            identity=len(nodes)
            nodes.append(dict(id=identity,parent=parent,key=str(key),ordinal=ordinal,raw=value))
            for i,(child_key,child) in enumerate(members(value)):visit(child,identity,child_key,i)
        visit(bundle['value'],-1,'',0)
        bundle['nodes']=nodes
    return result
