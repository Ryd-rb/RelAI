-- Status is fixed product copy. Never echo tool arguments, summaries, or errors.
WITH current_jobs AS (
    SELECT j.id, j.session, j.kind, j.state, j.reason, j.attempt, j.rowid AS sequence,
           CASE WHEN j.kind='tool' THEN j.input->>'$.arguments.tool' END AS tool,
           CASE WHEN j.kind='tool' THEN j.input->>'$.operation' END AS operation
      FROM jobs j JOIN requests r ON r.id=j.request
    WHERE r.id=?1 AND r.project=?2 AND j.project=r.project AND j.revision=r.revision
), active AS (
    SELECT kind,state,tool,operation FROM current_jobs
    WHERE state IN ('executing','running','sealing','sealed','queued','publishing')
    ORDER BY CASE state WHEN 'executing' THEN 0 WHEN 'running' THEN 1
        WHEN 'sealing' THEN 2 WHEN 'sealed' THEN 3 WHEN 'queued' THEN 4 ELSE 5 END,sequence
    LIMIT 1
)
SELECT
    CASE WHEN EXISTS (SELECT 1 FROM open_chat_questions q WHERE q.project=r.project AND q.request=r.id)
          THEN NULL
         WHEN EXISTS (SELECT 1 FROM actions a JOIN current_jobs j ON j.id=a.job WHERE
           (a.state='awaiting_acceptance' OR (a.state='evidence' AND EXISTS (SELECT 1 FROM branches b WHERE b.report=a.id AND b.accepted IS NULL)
             AND NOT EXISTS (SELECT 1 FROM questions q WHERE q.session=j.session AND q.revision=r.revision AND q.state!='answered'))))
          THEN NULL
         WHEN EXISTS (SELECT 1 FROM current_jobs j WHERE state='held' AND EXISTS(SELECT 1 FROM attempts a WHERE a.id=j.attempt AND a.state='rejected'))
          THEN 'The model response was received but rejected. Retry delivery to request another response.'
         WHEN EXISTS (SELECT 1 FROM current_jobs j WHERE state='uncertain'
             OR (state='held' AND EXISTS (SELECT 1 FROM attempts a WHERE a.id=j.attempt AND a.state='uncertain')))
          THEN 'This request was interrupted. Review before retrying.'
         WHEN EXISTS (SELECT 1 FROM current_jobs WHERE state='held' AND reason='Selected source access changed')
          THEN 'File access changed. Select the file again to continue.'
         WHEN EXISTS (SELECT 1 FROM current_jobs WHERE state='held' AND reason='Interrupted deterministic job')
          THEN 'A project operation was interrupted. Retry delivery to continue.'
         WHEN EXISTS (SELECT 1 FROM current_jobs WHERE state='held' AND kind='tool')
          THEN 'A project operation stopped. Retry delivery to continue.'
         WHEN EXISTS (SELECT 1 FROM current_jobs WHERE state='held')
          THEN 'The model response could not be completed. Retry delivery to continue.'
         WHEN EXISTS (SELECT 1 FROM actions a JOIN current_jobs j ON j.id=a.job WHERE a.state='held')
          THEN 'This request needs updated instructions.'
         WHEN EXISTS (SELECT 1 FROM active WHERE kind='tool') THEN (
           SELECT CASE
             WHEN tool IN ('fs.read','document.read','sheet.read','artifact.read','media.read')
               THEN CASE state WHEN 'queued' THEN 'Preparing to read files…' ELSE 'Reading the selected files…' END
             WHEN tool IN ('fs.list','fs.find','fs.stat','fs.count','fs.digest','archive.list','project.info')
               THEN CASE state WHEN 'queued' THEN 'Preparing to inspect the project…' ELSE 'Inspecting the project…' END
             WHEN tool IN ('fs.search','project.semantic_find','code.symbols','code.definition','code.references','code.diagnostics')
               THEN CASE state WHEN 'queued' THEN 'Preparing to search the project…' ELSE 'Searching the project…' END
             WHEN tool IN ('web.fetch','web.search','repo.issue_read','repo.pr_read','resource.read','connector.call')
               THEN CASE state WHEN 'queued' THEN 'Preparing an online lookup…' ELSE 'Looking up online information…' END
             WHEN tool IN ('project.lookup','plan.get','task.list','task.get','session.list','session.get','job.status','job.receipt','job.trace','dictionary.find','dictionary.get','dictionary.expand','skill.find','skill.read','tool.find','tool.inspect','workflow.find','data.read_query','resource.find')
               THEN CASE state WHEN 'queued' THEN 'Preparing a project lookup…' ELSE 'Looking up project information…' END
             WHEN tool IN ('verify.format','verify.lint','verify.build','verify.test','verify.benchmark')
               THEN CASE state WHEN 'queued' THEN 'Preparing project checks…' ELSE 'Checking the project…' END
             WHEN tool='patch.propose'
               THEN CASE state WHEN 'queued' THEN 'Preparing a patch for review…' ELSE 'Checking the patch against the current files…' END
             WHEN operation IN ('tool_test','tool_qualify') THEN 'Checking a project tool…'
             WHEN operation='tool_propose' THEN 'Preparing a project tool…'
             ELSE CASE state WHEN 'queued' THEN 'Preparing the next project step…' ELSE 'Running the next project step…' END
           END FROM active)
         WHEN EXISTS (SELECT 1 FROM active WHERE state='running') THEN 'Waiting for the model response…'
         WHEN EXISTS (SELECT 1 FROM active WHERE state IN ('queued','sealing','sealed')) THEN 'Preparing the model request…'
         WHEN EXISTS (SELECT 1 FROM active WHERE state='publishing') THEN 'Saving the result…'
         ELSE 'Waiting for a required step to finish.' END AS status
FROM requests r WHERE r.id=?1 AND r.project=?2 AND r.state='open'
