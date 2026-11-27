WITH review AS (
    SELECT c.files,e.state,e.digest,e.id,coalesce(e.error,e.result->>'$.native.error') AS error,
        c.revision=r.revision AND r.state IN ('open','completed') AS current,
        e.request->>'$.checkout.revision' AS head,
        e.request->>'$.checkout.reference' AS head_ref
    FROM chat_patch_reviews c
    JOIN file_effects e ON e.id=c.effect AND e.project=?1
    JOIN requests r ON r.id=c.request AND r.project=e.project
    WHERE c.effect=?2
), controls AS (
    SELECT 0 AS sort,json_object('id',?3||'/title','kind','heading','section',?3,
        'label',CASE
            WHEN state='completed' THEN 'Patch applied'
            WHEN state='planned' AND current THEN 'Patch ready for review'
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
    SELECT 4+CAST(f.key AS INTEGER),json_object('id',?3||'/file/'||f.key,'kind','detail',
        'section',?3,'label',(f.value->>'$.path')||CASE WHEN f.value->>'$.created'=1 THEN ' · New file' ELSE '' END,
        'value',f.value->>'$.diff','lines',9)
    FROM review,json_each(review.files) f WHERE ?4
    UNION ALL
    SELECT 21,json_object('id',?3||'/approve','kind','button','role','primary','section',?3,
        'label','Approve and apply patch','width',225,
        'event',json_object('action','command','spec',json_object(
            'type','button','label','Approve and apply patch','op','chat_patch_apply',
            'fixed',json_object('effect',id,'digest',digest))))
    FROM review WHERE ?4 AND state='planned' AND current
)
SELECT control AS "control:json" FROM controls ORDER BY sort
