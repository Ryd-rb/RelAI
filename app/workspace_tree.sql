WITH RECURSIVE tree(path, s, nav, dock, history, section) AS (
    SELECT printf('%04d', key), value, NULL, 0, value->>'$.type'='feed',
        CASE WHEN ?5=1 AND value->>'$.dockable'=1 THEN 'recent-projects' ELSE value->>'$.section' END
    FROM json_each(?1)
    WHERE value->>'$.pane' IS NULL OR value->>'$.pane'=?6
    UNION ALL
    SELECT tree.path||'.'||printf('%04d', c.key), c.value,
        CASE WHEN tree.s->>'$.type'='nav' THEN tree.path END,
        coalesce(tree.s->>'$.dockable',0), tree.history, coalesce(c.value->>'$.section',tree.section)
    FROM tree JOIN json_each(tree.s,'$.items') c
    WHERE (c.value->>'$.pane' IS NULL OR c.value->>'$.pane'=?6)
        AND (tree.s->>'$.type'<>'fold' OR json_extract(?2,'$."'||?3||'/'||coalesce(tree.s->>'$.foldKey',tree.path)||'"')=1)
        AND (coalesce(tree.s->>'$.dockable',0)=0 OR json_array_length(tree.s,'$.items')<=5 OR c.value->>'$.navigate.project' IS NULL)
        AND (tree.s->>'$.type'<>'feed' OR CAST(c.key AS INTEGER) BETWEEN
            json_array_length(tree.s,'$.items')-12*(1+min(?4,max(0,(json_array_length(tree.s,'$.items')-1)/12)))
            AND json_array_length(tree.s,'$.items')-1-12*min(?4,max(0,(json_array_length(tree.s,'$.items')-1)/12)))
)
SELECT path, json_set(json_remove(s,'$.items'),'$.itemCount',json_array_length(s,'$.items'),'$.tabs',
    json(CASE WHEN s->>'$.dockable'=1 AND json_array_length(s,'$.items')>5 THEN
        (SELECT json_group_array(json_object('label',coalesce(p.value->>'$.shortLabel',p.value->>'$.label'),'value',p.value->>'$.navigate.project'))
        FROM json_each(tree.s,'$.items') p WHERE p.value->>'$.navigate.project' IS NOT NULL) END)) AS "spec:json",
    nav, dock, section
FROM tree ORDER BY CASE WHEN ?5=1 AND (nav IS NOT NULL OR s->>'$.type'='nav') THEN 2 ELSE history END, path
