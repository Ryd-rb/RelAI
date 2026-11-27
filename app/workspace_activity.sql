WITH page AS (
SELECT e.cursor, e.kind,
    json_set(e.payload,'$.occurred_utc',strftime('%Y-%m-%d %H:%M:%S UTC',t.occurred,'unixepoch')) AS record,
    coalesce(strftime('%Y-%m-%d %H:%M:%S UTC',t.occurred,'unixepoch'),'Time unknown') || ' · ' || e.kind ||
    coalesce('   ·   ' || nullif(substr(replace(replace(coalesce(nullif(e.payload->>'$.error',''),e.payload->>'$.text',
      e.payload->>'$.payload.question',e.payload->>'$.summary',e.payload->>'$.reason',
      e.payload->>'$.payload.findings',''),char(10),' '),char(13),' '),1,110),''),'') AS label
FROM events e LEFT JOIN event_times t ON t.cursor=e.cursor
WHERE e.project=?1 AND (?3 IS NULL OR e.cursor<CAST(?3 AS INTEGER)) AND (
 ?2='Timeline'
 OR (e.kind='ASK' AND EXISTS(SELECT 1 FROM open_chat_questions q WHERE q.project=e.project AND q.id=e.payload->>'$.action'))
 OR (EXISTS (SELECT 1 FROM pending_chat_answers p
       WHERE p.project=e.project AND (
         (e.kind='RESULT' AND p.answer_id=e.payload->>'$.action')
         OR (e.kind='Final result' AND p.request=e.payload->>'$.request'
           AND EXISTS(SELECT 1 FROM finals f WHERE f.request=p.request AND f.action=p.answer_id)
           AND NOT EXISTS(SELECT 1 FROM events prior WHERE prior.project=e.project
             AND prior.kind='RESULT' AND prior.payload->>'$.action'=p.answer_id)))))
 OR (e.kind='RESULT' AND EXISTS(SELECT 1 FROM actions a
       JOIN jobs j ON j.id=a.job
       JOIN requests r ON r.id=j.request AND r.revision=j.revision AND r.state='open'
       WHERE j.project=e.project AND a.id=e.payload->>'$.action' AND a.state='evidence'
         AND EXISTS(SELECT 1 FROM branches b WHERE b.report=a.id AND b.accepted IS NULL)
         AND NOT EXISTS(SELECT 1 FROM questions q WHERE q.request=j.request
           AND q.revision=j.revision AND q.session=j.session AND q.state!='answered')))
 OR (EXISTS(SELECT 1 FROM jobs j JOIN requests r ON r.id=j.request AND r.revision=j.revision AND r.state='open' WHERE j.project=e.project AND j.id=e.payload->>'$.job' AND j.state IN ('held','uncertain')) AND (e.kind='Job held' OR lower(e.kind) LIKE '%uncertain%'))
 OR (e.kind='Action held' AND EXISTS(SELECT 1 FROM actions a JOIN jobs j ON j.id=a.job JOIN requests r ON r.id=j.request AND r.revision=j.revision AND r.state='open' WHERE j.project=e.project AND a.id=e.payload->>'$.action' AND a.state='held'))
)
ORDER BY e.cursor DESC LIMIT 101
)
SELECT CAST(cursor AS TEXT) AS value,kind,record AS "record:json",label,
       (SELECT count(*)>100 FROM page) AS has_older
FROM page ORDER BY cursor DESC LIMIT 100
