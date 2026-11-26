WITH ranked AS (
    SELECT id,rowid AS r,row_number() OVER (ORDER BY rowid) AS n,
        max(rowid) OVER () AS latest
    FROM snapshots WHERE project=?1
)
SELECT id,printf('Snapshot %d%s',n,CASE WHEN r=latest THEN ' · latest' ELSE '' END) AS label
FROM ranked
WHERE id=?2 OR r IN (SELECT r FROM ranked ORDER BY r DESC LIMIT 64)
ORDER BY r DESC
