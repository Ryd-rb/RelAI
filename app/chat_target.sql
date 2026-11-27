-- Resolve a draft only against the selected project's current conversation.
WITH available AS (
    SELECT id,prompt FROM open_chat_questions WHERE project=?1
), recent AS (
    SELECT request FROM submissions WHERE project=?1 ORDER BY rowid DESC LIMIT 1
), targets(priority, binding, hint, topic_request) AS (
    SELECT 1, json_object('reply',id),'Replying to “'||substr(prompt,1,64)||CASE WHEN length(prompt)>64 THEN '…' ELSE '' END||'”',NULL
      FROM available WHERE ?6 IS NULL AND (id=?2 OR (?2 IS NULL AND ?3 IS NULL AND ?4 IS NULL AND ?5 IS NOT 1 AND (SELECT count(*) FROM available)=1))
    UNION ALL
    SELECT 2,json_object('steer',id),'Updating',id FROM requests WHERE project=?1 AND id=?3 AND state='open' AND ?6 IS NULL
    UNION ALL
    SELECT CASE WHEN ?4 IS NULL AND ?6 IS NULL THEN 0 ELSE 3 END,
      json_object('follow',f.action),'Following up on',r.id FROM finals f JOIN requests r ON r.id=f.request
      WHERE r.project=?1 AND r.state='completed' AND (f.released=1 OR EXISTS (
        SELECT 1 FROM events e WHERE e.project=r.project AND e.kind='RESULT' AND e.payload->>'$.action'=f.action))
        AND ((?6 IS NOT NULL AND f.action=?6 AND (?3 IS NULL OR r.id=?3) AND ?2 IS NULL AND ?4 IS NULL)
          OR (?6 IS NULL AND (f.action=?4 OR (?2 IS NULL AND ?3 IS NULL AND ?4 IS NULL AND ?5 IS NOT 1
          AND f.request=(SELECT request FROM recent)))))
    UNION ALL
    SELECT 4,json_object(),'Choose a question to answer, or start a new topic.',NULL
      WHERE ?6 IS NULL AND ?2 IS NULL AND ?3 IS NULL AND ?4 IS NULL AND ?5 IS NOT 1
        AND (SELECT count(*) FROM available)>1
    UNION ALL
    SELECT CASE WHEN ?6 IS NULL THEN 0 ELSE 5 END,
      json_object('steer',p.request,'revise',p.answer_id),'Revising',p.request
      FROM pending_chat_answers p WHERE p.project=?1 AND (
        (p.answer_id=?6 AND (?3 IS NULL OR p.request=?3) AND ?2 IS NULL AND ?4 IS NULL) OR
        (?2 IS NULL AND ?3 IS NULL AND ?4 IS NULL AND ?6 IS NULL AND ?5 IS NOT 1
         AND p.request=(SELECT request FROM recent)
         AND (SELECT count(*) FROM pending_chat_answers WHERE project=?1 AND request=p.request)=1
         AND NOT EXISTS (SELECT 1 FROM questions q JOIN requests own ON own.id=q.request
           WHERE own.project=?1 AND own.id=p.request AND q.revision=own.revision
             AND q.state!='answered')))
), selected AS (SELECT * FROM targets ORDER BY priority LIMIT 1),
named AS (
    SELECT selected.*, (SELECT t.topic
      FROM chat_topics t WHERE t.request=selected.topic_request AND t.project=?1) AS topic
      FROM selected
)
SELECT binding AS "bind:json", CASE WHEN topic_request IS NULL THEN hint
         WHEN topic IS NULL THEN hint||' this request'
         ELSE hint||' “'||topic||'”' END AS hint,
       CASE WHEN priority=4 THEN 'true' END AS "unavailable:json",
       CASE WHEN priority=4 THEN 'true' END AS "choose_question:json",
       CASE WHEN (json_extract(binding,'$.revise') IS NOT NULL OR json_extract(binding,'$.follow') IS NOT NULL)
         AND EXISTS (SELECT 1 FROM available a JOIN questions q ON q.id=a.id
           WHERE q.request<>named.topic_request)
         THEN 'true' END AS "other_questions:json"
  FROM named
