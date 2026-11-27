WITH ranked AS (
    SELECT id,rowid AS r,row_number() OVER (ORDER BY rowid) AS n,
        max(rowid) OVER () AS latest
    FROM snapshots WHERE project=?1
), page AS (
    SELECT r FROM ranked
    WHERE ?4 IS NULL OR CASE WHEN ?5='newer' THEN r>?4 ELSE r<?4 END
    ORDER BY CASE WHEN ?5='newer' THEN r END ASC, r DESC
    LIMIT 64
), bounds AS (
    SELECT min(r) AS lo,max(r) AS hi FROM page
)
SELECT id,printf('Snapshot %d%s',n,CASE WHEN r=latest THEN ' · latest' ELSE '' END) AS label,
       CASE WHEN lo>(SELECT min(r) FROM ranked) THEN lo END AS older,
       CASE WHEN hi<latest THEN hi END AS newer
FROM ranked CROSS JOIN bounds
WHERE id IN (?2,?3) OR r IN (SELECT r FROM page)
ORDER BY r DESC
