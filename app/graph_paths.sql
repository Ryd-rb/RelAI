SELECT substr('                                                ',1,2*(length(p)-length(replace(p,'/','')))) || p || CASE WHEN kind='directory' THEN ' / · Folder' ELSE '' END AS label,
    p AS value
FROM (SELECT value->>'$.path' AS p,value->>'$.kind' AS kind FROM json_each(?1))
WHERE ?2='' OR instr(lower(p),lower(?2))>0
ORDER BY p
