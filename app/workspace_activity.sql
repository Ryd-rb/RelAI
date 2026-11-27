WITH page AS MATERIALIZED (
SELECT e.cursor, e.kind, e.payload,
    strftime('%Y-%m-%d %H:%M:%S UTC',t.occurred,'unixepoch') AS occurred
FROM events e LEFT JOIN event_times t ON t.cursor=e.cursor
WHERE e.project=?1 AND (?3 IS NULL OR e.cursor<CAST(?3 AS INTEGER))
 AND (?4 IS NULL OR e.cursor<=CAST(?4 AS INTEGER)) AND (
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
), tools AS (
    SELECT *,
        CASE kind WHEN 'RUN_TOOL' THEN payload->>'$.payload.tool'
             WHEN 'Tool receipt' THEN payload->>'$.tool'
             WHEN 'Tool outcome' THEN payload->>'$.receipt.tool' END AS tool,
        CASE kind WHEN 'RUN_TOOL' THEN
            CASE WHEN json_valid(payload->>'$.payload.arguments_json')
                 THEN payload->>'$.payload.arguments_json' END
             WHEN 'Tool receipt' THEN payload->'$.arguments'
             WHEN 'Tool outcome' THEN payload->'$.receipt.arguments' END AS tool_args
    FROM page
), display AS (
    SELECT *,
        CASE kind WHEN 'RUN_TOOL' THEN 'Tool requested' WHEN 'ASK' THEN 'Question'
             WHEN 'RESULT' THEN 'Result ready' ELSE kind END AS title,
        coalesce(nullif(payload->>'$.error',''),nullif(payload->>'$.receipt.error',''),
            nullif(payload->>'$.reason',''),nullif(payload->>'$.text',''),
            nullif(payload->>'$.payload.question',''),nullif(payload->>'$.summary',''),
            nullif(payload->>'$.payload.summary',''),nullif(payload->>'$.payload.findings',''),
            nullif(payload->>'$.result.summary',''),
            CASE kind WHEN 'Project opened' THEN payload->>'$.root'
                 WHEN 'Attempt started' THEN
                    (payload->>'$.preset.model')||' · '||(payload->>'$.preset.effort')||' reasoning'
            END) AS message,
        CASE WHEN tool IN ('fs.read','fs.list','fs.find','fs.search','fs.stat','fs.count','fs.digest')
            AND tool_args->>'$.host_path' IS NULL
            THEN nullif(tool_args->>'$.path','') END AS path
    FROM tools
), details AS (
    SELECT *,tool||coalesce(' · '||path,'') AS tool_label,coalesce(occurred,'Time unknown') AS time_label
    FROM display
)
SELECT CAST(cursor AS TEXT) AS value,title AS kind,time_label,tool_label,
       CASE WHEN length(message)>1500 THEN substr(message,1,1500)||'…' ELSE message END AS summary,
       payload AS record,
       time_label||' · '||title||coalesce(' · '||tool_label,'')||
           coalesce(' · '||nullif(substr(replace(replace(message,char(10),' '),char(13),' '),1,110),''),'') AS label,
       (SELECT count(*)>100 FROM page) AS has_older
FROM details ORDER BY cursor DESC LIMIT 100
