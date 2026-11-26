SELECT 'No comparison' AS label,'' AS value
UNION ALL
SELECT coalesce(value->>'$.label',value->>'$.id'),value->>'$.id'
FROM json_each(?1)
WHERE value->>'$.id'<>?2
