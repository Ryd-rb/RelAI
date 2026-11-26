SELECT
    CASE WHEN EXISTS (SELECT 1 FROM jobs j WHERE j.request=r.id AND j.revision=r.revision AND j.state IN ('held','uncertain'))
          THEN 'This request is paused.'
         WHEN EXISTS (SELECT 1 FROM actions a JOIN jobs j ON j.id=a.job WHERE j.request=r.id AND j.revision=r.revision AND a.state='held')
          THEN 'This request needs updated instructions.'
         WHEN EXISTS (SELECT 1 FROM questions q WHERE q.request=r.id AND q.revision=r.revision AND q.state='open'
           AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.request=r.id AND j.revision=r.revision AND j.kind='intake'
             AND j.input->>'$.reply'=q.id AND j.state IN ('queued','sealing','sealed','running','waiting')))
          THEN NULL
         WHEN EXISTS (SELECT 1 FROM actions a JOIN jobs j ON j.id=a.job WHERE j.request=r.id AND j.revision=r.revision AND
           (a.state='awaiting_acceptance' OR (a.state='evidence' AND EXISTS (SELECT 1 FROM branches b WHERE b.report=a.id AND b.accepted IS NULL))))
          THEN NULL
         WHEN EXISTS (SELECT 1 FROM jobs j WHERE j.request=r.id AND j.revision=r.revision AND j.state IN ('queued','sealing','sealed','running','publishing'))
          THEN 'Working…'
         ELSE 'Waiting for the next step.' END AS status
FROM requests r WHERE r.id=?1 AND r.project=?2 AND r.state='open'
