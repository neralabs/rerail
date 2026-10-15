SELECT ROW_NUMBER() OVER(PARTITION BY a.object_id ORDER BY a.object_id)
FROM sys.objects a
CROSS JOIN sys.objects b
CROSS JOIN sys.objects c
