-- Resolve an addressed reply or follow-up only inside the current project.
WITH available AS (
    SELECT q.id, q.body->>'$.question' AS prompt FROM questions q
    JOIN requests r ON r.id=q.request AND r.revision=q.revision
    WHERE r.project=?1 AND r.state='open' AND q.state='open'
      AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.request=r.id AND j.revision=r.revision
        AND j.kind='intake' AND j.input->>'$.reply'=q.id AND j.state IN ('queued','sealing','sealed','running','waiting','held','uncertain'))
), recent AS (
    SELECT request FROM submissions WHERE project=?1 ORDER BY rowid DESC LIMIT 1
), targets(priority, binding, hint) AS (
    SELECT 1, json_object('reply',id),'Replying to “'||substr(prompt,1,64)||CASE WHEN length(prompt)>64 THEN '…' ELSE '' END||'”'
      FROM available WHERE id=?2 OR (?2 IS NULL AND ?3 IS NULL AND ?4 IS NULL AND ?5 IS NOT 1 AND (SELECT count(*) FROM available)=1)
    UNION ALL
    SELECT 2,json_object('steer',id),'Updating “'||substr(intent,1,64)||'”' FROM requests WHERE project=?1 AND id=?3 AND state='open'
    UNION ALL
    SELECT 3,json_object('follow',f.action),'Following up on “'||substr(r.intent,1,64)||CASE WHEN length(r.intent)>64 THEN '…' ELSE '' END||'”' FROM finals f JOIN requests r ON r.id=f.request
      WHERE r.project=?1 AND r.state='completed' AND (f.released=1 OR EXISTS (
        SELECT 1 FROM events e WHERE e.project=r.project AND e.kind='RESULT' AND e.payload->>'$.action'=f.action))
        AND (f.action=?4 OR (?2 IS NULL AND ?3 IS NULL AND ?4 IS NULL AND ?5 IS NOT 1
        AND f.request=(SELECT request FROM recent) AND NOT EXISTS (SELECT 1 FROM requests WHERE project=?1 AND state='open')))
)
SELECT binding AS "bind:json", CASE WHEN priority=3 AND ?4 IS NULL THEN NULL ELSE hint END AS hint FROM targets ORDER BY priority LIMIT 1
