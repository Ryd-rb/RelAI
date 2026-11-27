WITH refs AS (
 SELECT value->'$.term' AS ref,min(CAST(key AS INTEGER)) AS position,
        value->>'$.text' AS spelling
 FROM json_each(?1) WHERE value->'$.term' IS NOT NULL
 GROUP BY value->>'$.term.id',value->>'$.term.revision',value->>'$.term.dictionary_id'
)
SELECT json(ref) AS "term:json",
 coalesce(e.name,spelling,'Term')||' · '||coalesce(e.kind,'unknown kind')||
 ' · '||coalesce(d.name,'unavailable dictionary')||' · definition' AS label
FROM refs LEFT JOIN entries e ON EXISTS(SELECT 1 FROM dictionary_links l JOIN dictionaries visible ON visible.id=l.dictionary
  WHERE l.project=?2 AND l.enabled=1 AND visible.archived=0 AND l.dictionary=ref->>'$.dictionary_id')
 AND e.id=ref->>'$.id' AND e.revision=ref->>'$.revision'
 AND e.dictionary=ref->>'$.dictionary_id'
LEFT JOIN dictionaries d ON d.id=e.dictionary
ORDER BY position
