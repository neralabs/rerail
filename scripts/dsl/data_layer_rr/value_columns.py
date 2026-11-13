"""Capture primitive values and container positions as typed columns, without coercion."""
import struct


def columns(value):
    names=['Kinds','Texts','Parents','Fields','Ordinals','Enter','Exit']
    result={'pv'+name:[]for name in names};clock=0
    def visit(value,parent=-1,field='',ordinal=0,*,pair=False):
        nonlocal clock
        index=len(result['pvKinds']);kind='pair'if pair else type(value).__name__
        text=value if kind=='str'else str(value)if kind=='int'else ('true'if value else 'false')if kind=='bool'else struct.pack('>d',value).hex()if kind=='float'else ''
        values=[kind,text,str(parent),field,str(ordinal),str(clock),''];clock+=1
        for name,item in zip(names,values):result['pv'+name].append(item)
        if pair:
            visit(value[0],index,'key',0);visit(value[1],index,'value',1)
        elif isinstance(value,dict):
            for i,item in enumerate(value.items()):visit(item,index,'',i,pair=True)
        elif isinstance(value,(list,tuple)):
            for i,item in enumerate(value):visit(item,index,'',i)
        result['pvExit'][index]=str(clock);clock+=1
        return index
    visit(value)
    return result
