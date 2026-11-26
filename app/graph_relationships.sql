SELECT json_group_array(json(value)) AS "items:json"
FROM (
    SELECT value FROM json_each(?1)
    WHERE ?2='' OR instr(lower(coalesce(value->>'$.source','')||' '||
        coalesce(value->>'$.target','')||' '||coalesce(value->>'$.name','')),lower(?2))>0
)
