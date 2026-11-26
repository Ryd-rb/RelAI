SELECT group_concat(coalesce(value->>'$.change',CASE
    WHEN value->>'$.old' IS NULL THEN 'added'
    WHEN value->>'$.new' IS NULL THEN 'removed'
    ELSE 'changed' END)||' · '||(value->>'$.path'),char(10)) AS text
FROM json_each(?)
