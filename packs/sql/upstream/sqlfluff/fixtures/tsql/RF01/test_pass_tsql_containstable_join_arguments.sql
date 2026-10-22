SELECT FT_TBL.Name,
       KEY_TBL.RANK
FROM Production.Product AS FT_TBL
     INNER JOIN CONTAINSTABLE(
         Production.Product,
         (Name, ProductNumber),
         'ISABOUT (frame WEIGHT (.8), wheel WEIGHT (.4), tire WEIGHT (.2))',
         LANGUAGE N'English',
         5
     ) AS KEY_TBL
         ON FT_TBL.ProductID = KEY_TBL.[KEY];
