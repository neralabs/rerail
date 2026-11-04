# SPDX-License-Identifier: Apache-2.0
"""Format passive annotation strings with Python's quote and escape conventions."""
from data_layer_six.common import S


def annotations():
    return S('annotation_reprs',r"""WITH RECURSIVE chars(record_id,ordinal,value,delimiter,position,piece) AS (
      SELECT record_id,ordinal,annotation,CASE WHEN instr(annotation,'''')>0 AND instr(annotation,'"')=0 THEN '"' ELSE '''' END,0,'' FROM actual_annotations
      UNION ALL SELECT record_id,ordinal,value,delimiter,position+1,
        CASE WHEN substr(value,position+1,1)='\' THEN '\\'
          WHEN substr(value,position+1,1)=delimiter THEN '\'||delimiter
          WHEN unicode(substr(value,position+1,1))=9 THEN '\t'
          WHEN unicode(substr(value,position+1,1))=10 THEN '\n'
          WHEN unicode(substr(value,position+1,1))=13 THEN '\r'
          WHEN substr(value,position+1,1)!=' ' AND rx_matches(substr(value,position+1,1),'^[\p{C}\p{Z}]$') THEN
            CASE WHEN unicode(substr(value,position+1,1))<256 THEN printf('\x%02x',unicode(substr(value,position+1,1)))
                 WHEN unicode(substr(value,position+1,1))<65536 THEN printf('\u%04x',unicode(substr(value,position+1,1)))
                 ELSE printf('\U%08x',unicode(substr(value,position+1,1))) END
          ELSE substr(value,position+1,1) END
      FROM chars WHERE position<length(value))
      SELECT record_id,ordinal,delimiter||group_concat(piece,'')||delimiter repr FROM
        (SELECT * FROM chars ORDER BY record_id,ordinal,position) GROUP BY record_id,ordinal,delimiter""",('record_id','ordinal'))
