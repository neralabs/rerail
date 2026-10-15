SELECT json_build_object(
    'name', 'ticket_status',
    'type', 'enum',
    'values', json_agg(status_name)
)
FROM unnest(enum_range(NULL::my_enum)) AS status_name;
