SELECT jsoncontent
FROM
    OPENROWSET (
        BULK (
            '/lake-raw/activity*.json'
            ,'/lake-raw/this_file_doesnt_exist.json'
        )
        ,DATA_SOURCE = 'datasource_lake'
    ) AS name
