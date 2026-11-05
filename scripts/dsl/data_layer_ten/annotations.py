# SPDX-License-Identifier: Apache-2.0
"""SQL implementation of matching's recursive _annotation_name contract."""
from .common import S


def stages():
    # Attribute names survive an unsupported prefix, whereas calls do not.
    # BinOp(BitOr) always contributes its separator, even with empty operands.
    return [
        S('annotation_nonempty', """WITH RECURSIVE names(__path,id) AS (
          SELECT __path,id FROM a WHERE kind IN ('Name','Attribute') OR kind='BinOp' AND op='BitOr'
          UNION SELECT parent.__path,parent.id FROM names n
          JOIN a child ON child.__path=n.__path AND child.id=n.id
          JOIN a parent ON parent.__path=child.__path AND parent.id=child.parent
          WHERE parent.kind='Subscript' AND child.field='value') SELECT * FROM names""", ('__path','id')),
        S('annotation_parts', """WITH RECURSIVE parts(__path,root,id,address) AS (
          SELECT n.__path,n.id,n.id,'' FROM a n LEFT JOIN a parent ON parent.__path=n.__path AND parent.id=n.parent
          WHERE n.field IN ('annotation','bases','type','decorator_list','func')
             OR n.field IN ('left','right') AND parent.kind='BinOp' AND parent.op='BitOr'
             OR n.field='value' AND parent.kind='Attribute'
          UNION ALL SELECT p.__path,p.root,child.id,p.address||CASE WHEN child.field='right' THEN '2' ELSE '0' END
          FROM parts p JOIN a parent ON parent.__path=p.__path AND parent.id=p.id
          JOIN a child ON child.__path=parent.__path AND child.parent=parent.id
          WHERE parent.kind IN ('Attribute','Subscript') AND child.field='value'
             OR parent.kind='BinOp' AND parent.op='BitOr' AND child.field IN ('left','right'))
          SELECT p.__path,p.root,p.address||'1' address,CASE
            WHEN n.kind='Name' THEN n.name
            WHEN n.kind='Attribute' THEN CASE WHEN EXISTS(
              SELECT 1 FROM a value JOIN annotation_nonempty known ON known.__path=value.__path AND known.id=value.id
              WHERE value.__path=n.__path AND value.parent=n.id AND value.field='value') THEN '.' ELSE '' END||n.name
            WHEN n.kind='BinOp' AND n.op='BitOr' THEN '|'
            ELSE '' END label FROM parts p JOIN a n ON n.__path=p.__path AND n.id=p.id""", ('__path','root')),
        S('annotation_labels', "SELECT __path,root id,group_concat(label,'') label FROM (SELECT * FROM annotation_parts ORDER BY __path,root,address) GROUP BY __path,root", ('__path','id')),
    ]
