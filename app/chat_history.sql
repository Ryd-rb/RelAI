-- Page by durable event cursors. Backend traffic never consumes a message slot.
WITH visible AS NOT MATERIALIZED (
    SELECT e.cursor, e.kind, e.payload, r.id AS request, r.state, r.revision
    FROM events e JOIN requests r ON r.id=e.payload->>'$.request' AND r.project=e.project
    WHERE e.project=?1 AND e.kind IN ('Submission','ASK','RESULT','Final result','Request stopped')
      AND (e.kind IN ('Submission','Request stopped')
        OR (e.kind='ASK' AND EXISTS (SELECT 1 FROM questions q WHERE q.id=e.payload->>'$.action'))
        OR (e.kind='RESULT' AND e.payload->>'$.state' IN ('awaiting_acceptance','evidence'))
        OR (e.kind='Final result' AND NOT EXISTS (
            SELECT 1 FROM finals f JOIN events proposed ON proposed.project=e.project AND proposed.kind='RESULT'
            AND proposed.payload->>'$.action'=f.action WHERE f.request=r.id)))
), older AS (
    SELECT * FROM visible WHERE ?3 IS NULL AND cursor<coalesce(?2,9223372036854775807)
    ORDER BY cursor DESC LIMIT 24
), newer AS (
    SELECT * FROM visible WHERE ?3 IS NOT NULL AND cursor>?3 ORDER BY cursor LIMIT 24
), page AS (SELECT * FROM older UNION ALL SELECT * FROM newer)
SELECT p.cursor, p.kind, p.request, p.state, CASE WHEN p.kind='Final result' THEN f.action ELSE p.payload->>'$.action' END AS action,
    CASE p.kind WHEN 'Submission' THEN p.payload->>'$.text'
      WHEN 'ASK' THEN p.payload->>'$.payload.question'
      WHEN 'RESULT' THEN p.payload->>'$.payload.findings'
      WHEN 'Final result' THEN p.payload->>'$.result.findings'
      WHEN 'Request stopped' THEN 'Request stopped.' END AS body,
    (SELECT json_group_array(json(value)) FROM json_each(p.payload,'$.parts')
      WHERE value->>'$.term' IS NOT NULL) AS "parts:json",
    (SELECT group_concat(value->>'$.display_name',char(10)) FROM json_each(p.payload,'$.attachments')) AS attachments,
    (SELECT group_concat(value,' · ') FROM json_each(p.payload,'$.payload.choices')) AS choices,
    CASE WHEN json_type(p.payload,'$.payload.choices')='array'
        AND json_array_length(p.payload,'$.payload.choices') BETWEEN 1 AND 6
        AND NOT EXISTS (SELECT 1 FROM json_each(p.payload,'$.payload.choices') WHERE type!='text' OR length(trim(value))=0 OR length(value)>48)
        THEN p.payload->'$.payload.choices' ELSE '[]' END AS "quick_choices:json",
    CASE WHEN p.kind='ASK' AND EXISTS (
        SELECT 1 FROM open_chat_questions oq WHERE oq.project=?1 AND oq.id=q.id) THEN 'reply'
      WHEN p.kind='RESULT' AND p.state='open' AND j.revision=p.revision THEN
        CASE WHEN a.state='awaiting_acceptance' THEN 'accept'
          WHEN a.state='evidence' AND EXISTS (SELECT 1 FROM branches b WHERE b.report=a.id AND b.accepted IS NULL)
            AND NOT EXISTS (SELECT 1 FROM questions q WHERE q.session=j.session AND q.revision=j.revision AND q.state!='answered')
            THEN 'review' END
      WHEN (p.kind='Final result' OR (p.kind='RESULT' AND f.action=a.id)) AND p.state='completed' THEN 'follow'
      END AS interaction,
    CASE WHEN p.kind='RESULT' AND p.payload->>'$.state'='awaiting_acceptance' THEN
      CASE WHEN f.action=a.id AND p.state='completed' THEN 'accepted'
           WHEN j.revision<p.revision THEN 'earlier' END
    END AS answer_status,
    CASE WHEN p.kind='Submission' AND p.state='open' AND p.payload->>'$.submission'=(
      SELECT s.id FROM submissions s WHERE s.request=p.request ORDER BY s.rowid DESC LIMIT 1)
      THEN 1 ELSE 0 END AS current_prompt,
    CASE WHEN p.state='open' AND NOT EXISTS (
      SELECT 1 FROM visible later WHERE later.payload->>'$.request'=p.request AND later.cursor>p.cursor
    ) THEN 1 ELSE 0 END AS request_tail,
    (SELECT min(cursor) FROM page) AS oldest, (SELECT max(cursor) FROM page) AS newest,
    EXISTS (SELECT 1 FROM visible WHERE cursor<(SELECT min(cursor) FROM page)) AS earlier,
    EXISTS (SELECT 1 FROM visible WHERE cursor>(SELECT max(cursor) FROM page)) AS later
FROM page p LEFT JOIN questions q ON q.id=p.payload->>'$.action'
LEFT JOIN actions a ON a.id=p.payload->>'$.action'
LEFT JOIN jobs j ON j.id=a.job
LEFT JOIN finals f ON f.request=p.request
ORDER BY p.cursor
