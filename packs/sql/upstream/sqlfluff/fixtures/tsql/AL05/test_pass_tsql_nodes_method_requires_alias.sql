SELECT Col.value('.', 'NVARCHAR(MAX)') AS V
FROM @Xml.nodes('/a') AS Tbl(Col);
