-- Complete B023 matching kernel. @kind@ placeholders use the active Python grammar.
-- All utility composition stays in SQL. The two native functions only read text
-- and match the fixed leaf pattern `functools.reduce`.
WITH RECURSIVE
loops AS MATERIALIZED (
  SELECT file,id,"last" FROM node WHERE kind IN
  (@for_statement@,@while_statement@,@list_comprehension@,@set_comprehension@,
   @dictionary_comprehension@,@generator_expression@)
),
lambda_chain(file,lambda,node) AS (
  SELECT file,id,id FROM node WHERE kind=@lambda@
  UNION ALL
  SELECT w.file,w.lambda,p.id FROM lambda_chain w
  JOIN node n ON n.file=w.file AND n.id=w.node
  JOIN node p ON p.file=n.file AND p.id=n.parent
  WHERE p.kind=@parenthesized_expression@
),
safe_lambda AS MATERIALIZED (
  SELECT DISTINCT w.file,w.lambda AS id FROM lambda_chain w
  JOIN node n ON n.file=w.file AND n.id=w.node
  JOIN node p ON p.file=n.file AND p.id=n.parent
  WHERE p.kind=@return_statement@
     OR (p.kind=@call@ AND EXISTS (
       SELECT 1 FROM node s WHERE s.file=n.file AND s.id=n.next
       AND s.kind IN (@argument_list@,@generator_expression@)))
     OR (p.kind=@argument_list@ AND EXISTS (
       SELECT 1 FROM node s WHERE s.file=p.file AND s.id=p.prev
       AND ((s.kind=@identifier@ AND spike_node_text(s.file,s.id) IN ('filter','reduce','map'))
         OR (s.kind=@attribute@ AND spike_is_reduce(s.file,s.id)))))
     OR (p.kind=@keyword_argument@ AND EXISTS (
       SELECT 1 FROM node s WHERE s.file=n.file AND s.id=n.prev
       AND spike_node_text(s.file,s.id)='=') AND EXISTS (
       SELECT 1 FROM node k JOIN node eq ON eq.file=k.file AND eq.id=k.next
       WHERE k.file=p.file AND k.parent=p.id AND k.kind=@identifier@
         AND spike_node_text(k.file,k.id)='key' AND spike_node_text(eq.file,eq.id)='='))
),
functions AS MATERIALIZED (
  SELECT file,id,"last",parent,kind FROM node n
  WHERE kind=@function_definition@ OR (kind=@lambda@ AND NOT EXISTS (
    SELECT 1 FROM safe_lambda s WHERE s.file=n.file AND s.id=n.id))
),
function_climb(file,fn,node) AS (
  SELECT file,id,parent FROM functions WHERE parent IS NOT NULL
  UNION ALL
  SELECT w.file,w.fn,n.parent FROM function_climb w
  JOIN node n ON n.file=w.file AND n.id=w.node
  WHERE n.parent IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM functions f WHERE f.file=w.file AND f.id=w.node)
),
function_loops AS MATERIALIZED (
  SELECT w.file,w.fn,l.id AS loop FROM function_climb w
  JOIN loops l ON l.file=w.file AND l.id=w.node
),
relevant_loops AS MATERIALIZED (
  SELECT DISTINCT l.file,l.id,l."last" FROM function_loops f
  JOIN loops l ON l.file=f.file AND l.id=f.loop
),
regions AS MATERIALIZED (
  SELECT l.* FROM relevant_loops l WHERE NOT EXISTS (
    SELECT 1 FROM relevant_loops outer_loop WHERE outer_loop.file=l.file
      AND outer_loop.id<l.id AND outer_loop."last">=l.id)
),
target_slot(file,node) AS (
  SELECT c.file,c.id FROM regions r
  CROSS JOIN node p ON p.file=r.file AND p.id>=r.id AND p.id<=r."last"
  CROSS JOIN node c ON c.file=p.file AND c.parent=p.id
  WHERE (p.kind IN (@assignment@,@augmented_assignment@) AND NOT EXISTS (
      SELECT 1 FROM node prev WHERE prev.file=c.file AND prev.id=c.prev AND prev."end">prev.start))
    OR (p.kind IN (@for_statement@,@for_in_clause@) AND EXISTS (
      SELECT 1 FROM node prev WHERE prev.file=c.file AND prev.id=c.prev
        AND spike_node_text(prev.file,prev.id)='for'))
  UNION
  SELECT c.file,c.id FROM target_slot t
  JOIN node p ON p.file=t.file AND p.id=t.node
  JOIN node c ON c.file=p.file AND c.parent=p.id
  WHERE p.kind IN (@pattern_list@,@tuple_pattern@,@list_pattern@,@list_splat_pattern@,
                  @tuple@,@list@,@parenthesized_expression@)
),
targets AS MATERIALIZED (
  SELECT n.file,n.id,n.parent,n.atom FROM target_slot t
  JOIN node n ON n.file=t.file AND n.id=t.node WHERE n.kind=@identifier@
),
assignment_climb(file,atom,target,node) AS (
  SELECT file,atom,id,parent FROM targets WHERE parent IS NOT NULL
  UNION
  SELECT w.file,w.atom,w.target,n.parent FROM assignment_climb w
  JOIN node n ON n.file=w.file AND n.id=w.node
  WHERE n.parent IS NOT NULL AND n.kind NOT IN (@function_definition@,@lambda@)
),
loop_assigns AS MATERIALIZED (
  SELECT w.file,l.id AS loop,w.atom,MIN(w.target) AS capture FROM assignment_climb w
  JOIN loops l ON l.file=w.file AND l.id=w.node
  GROUP BY w.file,l.id,w.atom
),
loop_choices AS (
  SELECT f.file,f.fn,a.atom,a.capture,ROW_NUMBER() OVER (
    PARTITION BY f.file,f.fn,a.atom ORDER BY a.loop DESC) AS choice
  FROM function_loops f JOIN loop_assigns a ON a.file=f.file AND a.loop=f.loop
),
demand AS MATERIALIZED (
  SELECT file,fn,atom,capture FROM loop_choices WHERE choice=1
),
store_slot(file,node) AS (
  SELECT file,node FROM target_slot
  UNION
  SELECT c.file,c.id FROM regions r
  CROSS JOIN node p ON p.file=r.file AND p.id>=r.id AND p.id<=r."last"
  CROSS JOIN node c ON c.file=p.file AND c.parent=p.id
  WHERE p.kind=@named_expression@ AND NOT EXISTS (
    SELECT 1 FROM node prev WHERE prev.file=c.file AND prev.id=c.prev AND prev."end">prev.start)
  UNION
  SELECT n.file,n.id FROM regions r
  CROSS JOIN node n ON n.file=r.file AND n.id>=r.id AND n.id<=r."last"
  JOIN node p ON p.file=n.file AND p.id=n.parent
  JOIN node g ON g.file=p.file AND g.id=p.parent
  WHERE n.kind=@as_pattern_target@ AND p.kind=@as_pattern@ AND g.kind=@with_item@
  UNION
  SELECT c.file,c.id FROM store_slot s
  JOIN node p ON p.file=s.file AND p.id=s.node
  JOIN node c ON c.file=p.file AND c.parent=p.id
  WHERE p.kind IN (@as_pattern_target@,@tuple@,@list@,@parenthesized_expression@,@list_splat_pattern@)
),
stores AS MATERIALIZED (
  SELECT n.file,n.id,n.atom FROM store_slot s
  CROSS JOIN node n ON n.file=s.file AND n.id=s.node WHERE n.kind=@identifier@
),
parameter_walk(file,fn,node) AS (
  SELECT DISTINCT d.file,d.fn,c.id FROM demand d
  JOIN node p ON p.file=d.file AND p.parent=d.fn
  JOIN node c ON c.file=p.file AND c.parent=p.id
  WHERE p.kind IN (@parameters@,@lambda_parameters@)
  UNION
  SELECT w.file,w.fn,c.id FROM parameter_walk w
  JOIN node p ON p.file=w.file AND p.id=w.node
  JOIN node c ON c.file=p.file AND c.parent=p.id
  WHERE (p.kind IN (@list_splat_pattern@,@dictionary_splat_pattern@) AND c.kind=@identifier@)
     OR (p.kind IN (@default_parameter@,@typed_parameter@,@typed_default_parameter@) AND NOT EXISTS (
       SELECT 1 FROM node prev WHERE prev.file=c.file AND prev.id=c.prev AND prev."end">prev.start))
),
parameters AS MATERIALIZED (
  SELECT w.file,w.fn,n.atom FROM parameter_walk w
  CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.kind=@identifier@
),
unbound AS MATERIALIZED (
  SELECT d.file,d.fn,d.atom,d.capture FROM demand d JOIN functions f ON f.file=d.file AND f.id=d.fn
  WHERE NOT EXISTS (SELECT 1 FROM parameters p WHERE p.file=d.file AND p.fn=d.fn AND p.atom=d.atom)
    AND NOT EXISTS (SELECT 1 FROM stores s WHERE s.file=d.file AND s.atom=d.atom AND s.id>f.id AND s.id<=f."last")
),
bodies AS MATERIALIZED (
  SELECT u.file,u.fn,u.atom,u.capture,b.id,b."last",v.thash FROM unbound u
  JOIN functions f ON f.file=u.file AND f.id=u.fn
  JOIN node b ON b.file=f.file AND b.parent=f.id
  JOIN node v ON v.file=u.file AND v.id=u.atom
  WHERE (f.kind=@function_definition@ AND b.kind=@block@)
     OR (f.kind=@lambda@ AND EXISTS (SELECT 1 FROM node prev
       WHERE prev.file=b.file AND prev.id=b.prev AND spike_node_text(prev.file,prev.id)=':'))
),
load_candidates AS (
  SELECT n.file,n.id,n.parent,n.prev,n.next,b.capture,ROW_NUMBER() OVER (
    PARTITION BY n.file,n.id ORDER BY b.id DESC) AS choice FROM bodies b
  CROSS JOIN node n ON n.file=b.file AND n.kind=@identifier@ AND n.thash=b.thash
    AND n.id>=b.id AND n.id<=b."last" AND n.atom=b.atom
),
loads AS MATERIALIZED (
  SELECT file,id,parent,prev,next,capture FROM load_candidates WHERE choice=1
),
matches AS (
  SELECT n.file,n.id,n.capture FROM loads n
  JOIN node p ON p.file=n.file AND p.id=n.parent
  LEFT JOIN node g ON g.file=p.file AND g.id=p.parent
  LEFT JOIN node gg ON gg.file=g.file AND gg.id=g.parent
  WHERE NOT EXISTS (SELECT 1 FROM stores s WHERE s.file=n.file AND s.id=n.id)
    AND p.kind<>@delete_statement@
    AND NOT (p.kind=@attribute@ AND EXISTS (SELECT 1 FROM node prev
      WHERE prev.file=n.file AND prev.id=n.prev AND spike_node_text(prev.file,prev.id)='.'))
    AND NOT (p.kind=@keyword_argument@ AND EXISTS (SELECT 1 FROM node nex
      WHERE nex.file=n.file AND nex.id=n.next AND spike_node_text(nex.file,nex.id)='='))
    AND NOT (p.kind IN (@function_definition@,@class_definition@,@parameters@,@lambda_parameters@,
      @dictionary_splat_pattern@,@dotted_name@,@aliased_import@,@global_statement@,@nonlocal_statement@,
      @keyword_pattern@,@splat_pattern@,@type_parameter@)
      OR (p.kind=@list_splat_pattern@ AND COALESCE(g.kind IN (@parameters@,@lambda_parameters@,@typed_parameter@),0))
      OR (p.kind=@as_pattern_target@ AND COALESCE(g.kind=@as_pattern@ AND gg.kind=@except_clause@,0))
      OR (p.kind=@as_pattern@ AND COALESCE(g.kind=@case_pattern@,0)))
    AND NOT (p.kind IN (@default_parameter@,@typed_parameter@,@typed_default_parameter@) AND NOT EXISTS (
      SELECT 1 FROM node prev WHERE prev.file=n.file AND prev.id=n.prev AND prev."end">prev.start))
)
SELECT 0,file,id,json_array(capture) FROM matches ORDER BY file,id
