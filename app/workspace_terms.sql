SELECT e.id AS value, e.name || '   ·   ' || d.name AS label, e.name, e.definition,
    json_object('id',e.id,'revision',e.revision,'dictionary_id',e.dictionary) AS "ref:json"
FROM entries e JOIN dictionaries d ON d.id=e.dictionary AND d.archived=0
JOIN dictionary_links l ON l.dictionary=d.id AND l.project=?1 AND l.enabled=1
WHERE e.revision=(SELECT max(latest.revision) FROM entries latest WHERE latest.id=e.id)
    AND instr(lower(e.name),lower(coalesce(?2,'')))>0
ORDER BY e.name COLLATE NOCASE,d.name COLLATE NOCASE,e.id LIMIT 80
