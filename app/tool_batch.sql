-- Only portable, read-only siblings from one admitted foreground envelope fan in.
-- The original envelope, action IDs, and tool-job receipts remain unchanged.
WITH source AS (
    SELECT s.*,json_array_length(s.receipt,'$.actions') AS expected
    FROM jobs t JOIN jobs s ON s.id=t.input->>'$.source'
    JOIN requests r ON r.id=s.request AND r.project=s.project
        AND r.revision=s.revision AND r.state='open'
    WHERE t.id=?1 AND t.project=s.project AND t.request=s.request
        AND t.revision=s.revision AND t.session=s.session AND t.role=s.role
        AND s.state='accepted' AND s.role!='workflow_improver'
        AND s.receipt->>'$.next'='wait_for'
        AND json_array_length(s.receipt,'$.actions') BETWEEN 2 AND 8
        AND NOT EXISTS (
            SELECT 1 FROM json_each(s.receipt,'$.actions') a
            WHERE a.value->>'$.type'!='RUN_TOOL'
                OR a.value->>'$.tool' NOT IN ('fs.list','fs.read','fs.search','tool.find')
                OR CASE WHEN json_valid(a.value->>'$.arguments_json')
                    THEN json_extract(a.value->>'$.arguments_json','$.host_path') IS NOT NULL
                    ELSE 1 END)
        AND NOT EXISTS (
            SELECT 1 FROM json_tree(coalesce(s.seal->'$.selection.delivery',s.input))
            WHERE key='host_path' AND type!='null')
        -- An upgrade must not redeliver a receipt already routed separately.
        AND NOT EXISTS (
            SELECT 1 FROM jobs m JOIN actions a ON a.id=m.input->>'$.action'
            WHERE a.job=s.id AND m.project=s.project AND m.request=s.request
                AND m.revision=s.revision AND m.session=s.session AND m.role=s.role
                AND m.kind='message' AND m.input->'$.tool_receipt' IS NOT NULL)
), siblings AS (
    SELECT t.* FROM jobs t JOIN source s ON t.project=s.project
        AND t.request=s.request AND t.revision=s.revision
        AND t.session=s.session AND t.role=s.role AND t.kind='tool'
        AND t.input->>'$.source'=s.id
), batch AS (
SELECT s.id AS source,
    CASE WHEN (SELECT count(*) FROM actions a WHERE a.job=s.id)=s.expected
        AND NOT EXISTS (SELECT 1 FROM actions a WHERE a.job=s.id AND
            (a.state!='executed' OR a.kind!='RUN_TOOL'
             OR (SELECT count(*) FROM siblings t WHERE t.input->>'$.action'=a.id
                 AND t.state='accepted' AND t.receipt IS NOT NULL
                 AND t.input->>'$.arguments.arguments.host_path' IS NULL)!=1))
        AND NOT EXISTS (SELECT 1 FROM siblings WHERE state NOT IN ('accepted','superseded'))
        AND NOT EXISTS (SELECT 1 FROM jobs m WHERE m.project=s.project
            AND m.request=s.request AND m.revision=s.revision AND m.session=s.session
            AND m.role=s.role AND m.kind='message' AND m.input->>'$.tool_batch_source'=s.id)
        THEN 1 ELSE 0 END AS ready
FROM source s
)
SELECT b.source,b.ready,
    CASE WHEN b.ready=1 THEN (SELECT coalesce(seal->'$.selection.delivery',input)
        FROM source) END AS "original_assignment:json",
    CASE WHEN b.ready=1 THEN (SELECT json_group_array(json(entry)) FROM (
        SELECT json_object('action',a.id,'job',t.id,'tool',a.payload->>'$.tool',
            'receipt',json(t.receipt)) AS entry
        FROM actions a JOIN siblings t ON t.input->>'$.action'=a.id
        WHERE a.job=b.source AND t.state='accepted' ORDER BY a.rowid
    )) END AS "tool_receipts:json"
FROM batch b
