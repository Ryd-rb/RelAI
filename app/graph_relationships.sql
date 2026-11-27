SELECT json_group_array(json(value)) AS "items:json"
FROM (
    SELECT value FROM json_each(?1)
    WHERE ?2='' OR instr(unicode_fold(coalesce(value->>'$.source','')||' '||
        coalesce(value->>'$.target','')||' '||coalesce(value->>'$.name','')),unicode_fold(?2))>0
)
