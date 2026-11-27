WITH review AS (
    SELECT c.files,e.state,e.digest,e.id,coalesce(e.error,e.result->>'$.native.error') AS error,
        c.revision=r.revision AND r.state IN ('open','completed') AS current,
        e.request->>'$.checkout.revision' AS head,
        e.request->>'$.checkout.reference' AS head_ref
    FROM chat_patch_reviews c
    JOIN file_effects e ON e.id=c.effect AND e.project=?1
    JOIN requests r ON r.id=c.request AND r.project=e.project
    WHERE c.effect=?2
), file_choices AS (
    SELECT CAST(f.key AS INTEGER) AS position,f.value->>'$.path' AS path,
        (f.value->>'$.path')||CASE WHEN f.value->>'$.created'=1 THEN ' · New file' ELSE '' END AS label,
        f.value->>'$.diff' AS diff
    FROM review,json_each(review.files) f WHERE ?4
), displayed AS (
    SELECT * FROM file_choices ORDER BY path IS NOT ?5,position LIMIT 1
), controls AS (
    SELECT 0 AS sort,json_object('id',?3||'/title','kind','heading','section',?3,
        'label',CASE
            WHEN state='completed' THEN 'Patch applied'
            WHEN state='planned' AND current THEN 'Patch needs a fresh request'
            WHEN state='planned' THEN 'Patch superseded'
            WHEN state IN ('authorized','executing') THEN 'Applying patch…'
            WHEN state='uncertain' THEN 'Patch outcome needs attention'
            WHEN state='conflict' THEN 'Patch no longer matches the checkout'
            ELSE 'Patch could not be applied' END) AS control
    FROM review
    UNION ALL
    SELECT 1,json_object('id',?3||'/scope','kind','label','role','muted','section',?3,
        'label',json_array_length(files)||CASE WHEN json_array_length(files)=1 THEN ' file' ELSE ' files' END
          ||' · '||coalesce(substr(head_ref,12)||' · ','')||'Git '||substr(head,1,12))
    FROM review
    UNION ALL
    SELECT 2,json_object('id',?3||'/error','kind','detail','role','body','caption',json('false'),
        'section',?3,'label','Patch error','value',error,'lines',3)
    FROM review WHERE NULLIF(error,'') IS NOT NULL
    UNION ALL
    SELECT 3,json_object('id',?3,'kind','button','role','quiet','section',?3,
        'label',CASE WHEN ?4 THEN 'Hide patch diff' ELSE 'Review patch' END,
        'expanded',json(CASE WHEN ?4 THEN 'true' ELSE 'false' END),
        'event',json_object('action','fold','key',?3))
    FROM review
    UNION ALL
    SELECT 4,json_object('id',?3||'/files','kind','select','section',?3,
        'label','File to review','value',path,
        'choices',json((SELECT json_group_array(json_object('value',path,'label',label) ORDER BY position)
                        FROM file_choices)),
        'event',json_object('action','workspace_select','scope',?3,'name','file'))
    FROM displayed WHERE (SELECT count(*) FROM file_choices)>1
    UNION ALL
    SELECT 5,json_object('id',?3||'/file/'||position,'kind','detail',
        'section',?3,'label',label,'value',diff,'lines',9)
    FROM displayed
)
SELECT control AS "control:json" FROM controls ORDER BY sort
