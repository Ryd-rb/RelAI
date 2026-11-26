SELECT CAST(cursor AS TEXT) AS value, kind, payload AS "record:json",
    kind || coalesce('   ·   ' || nullif(substr(replace(replace(coalesce(payload->>'$.text',
      payload->>'$.payload.question',payload->>'$.summary',payload->>'$.reason',
      payload->>'$.payload.findings',''),char(10),' '),char(13),' '),1,110),''),'') AS label
FROM events WHERE project=?1 AND (?2='Timeline' OR kind IN ('ASK','Request stopped')
    OR lower(kind) LIKE '%error%' OR lower(kind) LIKE '%failed%' OR lower(kind) LIKE '%held%'
    OR lower(kind) LIKE '%uncertain%' OR payload->>'$.state' IN ('held','failed','uncertain','awaiting_acceptance'))
ORDER BY cursor DESC LIMIT 100
