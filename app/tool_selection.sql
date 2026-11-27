WITH entries(item) AS (
    SELECT value FROM json_each(?1, '$.builtins')
    UNION ALL SELECT value FROM json_each(?1, '$.versions')
)
SELECT json(item) AS "entry:json",
    CASE WHEN item->>'$.id' IS NOT NULL OR
        (item->>'$.status' = 'active' AND item->>'$.runtime' = (?1->>'$.runtime'))
        THEN 'true' ELSE 'false' END AS "ready:json"
FROM entries
WHERE coalesce(item->>'$.id', item->>'$.name') = ?2
    AND (item->>'$.version') IS CASE WHEN item->>'$.id' IS NOT NULL THEN nullif(?3,'')
        ELSE coalesce(nullif(?3,''), (SELECT value->>'$.version' FROM json_each(?1,'$.defaults')
            WHERE value->>'$.name' = ?2 LIMIT 1)) END
    AND (item->>'$.id' IS NULL OR nullif(?3,'') IS NULL)
LIMIT 1
