WITH RECURSIVE
calls AS MATERIALIZED (SELECT n.file,n.id FROM node n WHERE n.kind=@call AND @roots),
attributes AS MATERIALIZED (SELECT c.file,c.id call,a.id FROM calls c JOIN node a ON a.file=c.file AND a.parent=c.id AND a.kind=@attribute),
attribute_children AS MATERIALIZED (SELECT a.call,n.file,n.id,n.kind,n.prev,n."last",a.id attribute FROM attributes a JOIN node n ON n.file=a.file AND n.parent=a.id),
receiver_roots AS MATERIALIZED (
 SELECT a.file,a.call,a.attribute,a.id node FROM attribute_children a
 WHERE EXISTS(SELECT 1 FROM node dot WHERE dot.file=a.file AND dot.parent=a.attribute AND dot.id>a."last" AND dot.kind<>@comment AND node_regex(@dot,dot.file,dot.id)
 AND NOT EXISTS(SELECT 1 FROM node between_node WHERE between_node.file=a.file AND between_node.parent=a.attribute AND between_node.id>a."last" AND between_node.id<dot.id AND between_node.kind<>@comment))
),
method_calls AS MATERIALIZED (SELECT DISTINCT a.file,a.call,a.attribute FROM attribute_children a JOIN node dot ON dot.file=a.file AND dot.id=a.prev WHERE a.kind=@identifier AND node_regex(@method,a.file,a.id) AND node_regex(@dot,dot.file,dot.id)),
arguments AS MATERIALIZED (SELECT c.file,c.id call,a.id FROM calls c JOIN node a ON a.file=c.file AND a.parent=c.id AND a.kind=@argument_list),
argument_nodes AS MATERIALIZED (SELECT a.call,a.id arguments,n.file,n.id,n.kind,n.prev FROM arguments a JOIN node n ON n.file=a.file AND n.parent=a.id),
noncomments AS MATERIALIZED (SELECT a.*,node_regex(@punct,a.file,a.id) punct,node_regex(@open,a.file,a.id) opening,node_regex(@comma,a.file,a.id) comma FROM argument_nodes a WHERE a.kind<>@comment),
positions AS MATERIALIZED (SELECT a.*,
 LAG(a.opening,1,0) OVER w first_position,
 LAG(a.comma,1,0) OVER w previous_comma,
 LAG(a.punct OR a.kind IN (@keyword_argument,@list_splat,@dictionary_splat),2,1) OVER w previous_previous_excluded,
 LAG(a.opening,3,0) OVER w third_previous_open
 FROM noncomments a WINDOW w AS(PARTITION BY a.file,a.arguments ORDER BY a.id)),
keyword_names AS MATERIALIZED (SELECT a.file,a.arguments,a.id keyword,CASE WHEN node_regex(@sep,n.file,n.id) THEN 1 WHEN node_regex(@maxsplit,n.file,n.id) THEN 2 END role
 FROM argument_nodes a JOIN node n ON n.file=a.file AND n.parent=a.id AND n.kind=@identifier JOIN node eq ON eq.file=n.file AND eq.id=n.next
 WHERE a.kind=@keyword_argument AND node_regex(@equals,eq.file,eq.id) AND (node_regex(@sep,n.file,n.id) OR node_regex(@maxsplit,n.file,n.id))),
keyword_values AS MATERIALIZED (SELECT k.file,k.arguments,k.role,v.id node FROM keyword_names k JOIN node v ON v.file=k.file AND v.parent=k.keyword JOIN node eq ON eq.file=v.file AND eq.id=v.prev WHERE node_regex(@equals,eq.file,eq.id)),
selected AS MATERIALIZED (
 SELECT file,arguments,role,node FROM keyword_values
 UNION ALL SELECT p.file,p.arguments,CASE WHEN p.first_position THEN 1 ELSE 2 END,p.id FROM positions p
 WHERE NOT p.punct AND p.kind NOT IN (@keyword_argument,@list_splat,@dictionary_splat)
 AND (p.first_position OR (p.previous_comma AND NOT p.previous_previous_excluded AND p.third_previous_open))
 AND NOT EXISTS(SELECT 1 FROM keyword_names k WHERE k.file=p.file AND k.arguments=p.arguments AND k.role=CASE WHEN p.first_position THEN 1 ELSE 2 END)
),
roots AS MATERIALIZED (SELECT file,node FROM receiver_roots UNION SELECT file,node FROM selected),
closure(file,node) AS (SELECT file,node FROM roots UNION SELECT n.file,n.id FROM closure d JOIN node p ON p.file=d.file AND p.id=d.node JOIN node n ON n.file=p.file AND n.parent=p.id WHERE p.kind IN (@parenthesized_expression,@unary_operator)),
parts AS MATERIALIZED (SELECT c.file,c.node,n.kind FROM closure c JOIN node n ON n.file=c.file AND n.id=c.node WHERE n.kind IN (@string,@concatenated_string)),
string_flags AS MATERIALIZED (SELECT p.file,p.node,
 EXISTS(SELECT 1 FROM node s WHERE s.file=p.file AND s.parent=p.node AND s.kind=@string_start AND node_regex(@badprefix,s.file,s.id))
 OR EXISTS(SELECT 1 FROM node part JOIN node s ON s.file=part.file AND s.parent=part.id AND s.kind=@string_start WHERE part.file=p.file AND part.parent=p.node AND part.kind=@string AND node_regex(@badprefix,s.file,s.id)) bad FROM parts p),
integers AS MATERIALIZED (SELECT c.file,c.node,v.truth FROM closure c JOIN node n ON n.file=c.file AND n.id=c.node AND n.kind=@integer JOIN relation_value_budget v ON v.term=@integer_value AND v.file=c.file AND v.node=c.node AND v.allowance=9223372036854775807),
helper(file,node,family) AS (
 SELECT file,node,1 FROM string_flags WHERE NOT bad
 UNION SELECT file,node,2 FROM string_flags WHERE NOT bad
 UNION SELECT c.file,c.node,2 FROM closure c JOIN node n ON n.file=c.file AND n.id=c.node WHERE n.kind=@none
 UNION SELECT file,node,3 FROM integers WHERE truth=1
 UNION SELECT p.file,p.id,CASE WHEN p.kind=@unary_operator THEN 4 ELSE h.family END
 FROM helper h JOIN node child ON child.file=h.file AND child.id=h.node JOIN node p ON p.file=child.file AND p.id=child.parent
 JOIN closure d ON d.file=p.file AND d.node=p.id
 WHERE p.kind=@parenthesized_expression OR (h.family=3 AND p.kind=@unary_operator AND EXISTS(SELECT 1 FROM node minus WHERE minus.file=p.file AND minus.parent=p.id AND node_regex(@minus,minus.file,minus.id)))
),
receiver_ok AS MATERIALIZED (SELECT DISTINCT r.file,r.call,r.attribute FROM receiver_roots r JOIN helper h ON h.file=r.file AND h.node=r.node AND h.family=1),
argument_ok AS MATERIALIZED (SELECT a.file,a.call FROM arguments a WHERE
 ((NOT EXISTS(SELECT 1 FROM selected s WHERE s.file=a.file AND s.arguments=a.id AND s.role=1) AND NOT EXISTS(SELECT 1 FROM keyword_names k WHERE k.file=a.file AND k.arguments=a.id AND k.role=1)) OR EXISTS(SELECT 1 FROM selected s JOIN helper h ON h.file=s.file AND h.node=s.node AND h.family=2 WHERE s.file=a.file AND s.arguments=a.id AND s.role=1))
 AND ((NOT EXISTS(SELECT 1 FROM selected s WHERE s.file=a.file AND s.arguments=a.id AND s.role=2) AND NOT EXISTS(SELECT 1 FROM keyword_names k WHERE k.file=a.file AND k.arguments=a.id AND k.role=2)) OR EXISTS(SELECT 1 FROM selected s JOIN helper h ON h.file=s.file AND h.node=s.node AND h.family IN (3,4) WHERE s.file=a.file AND s.arguments=a.id AND s.role=2)))
SELECT DISTINCT 0 clause,r.file,r.call node,'[]' captures FROM receiver_ok r JOIN method_calls m ON m.file=r.file AND m.call=r.call AND m.attribute=r.attribute JOIN argument_ok a ON a.file=r.file AND a.call=r.call ORDER BY r.file,r.call
