WITH delivery AS (
  SELECT ?1 AS base,?1||'/'||(?2->>'$.id') AS key,?2->>'$.id' AS job,
         ?2->>'$.state' AS state,?2->>'$.job_exhausted' AS job_exhausted,
         ?2->>'$.request_exhausted' AS request_exhausted,
         ?2->>'$.budget_row' AS budget_row,?3 IS 1 AS expanded
), choices(n,suffix,label,value,kind,width,event) AS (
  SELECT 1,'/uncertain',label,label,'label',NULL,NULL
  FROM (SELECT CASE WHEN request_exhausted=1
               THEN 'Delivery could not be confirmed.'
               ELSE 'Delivery could not be confirmed. Trying again may send the message again.'
           END AS label FROM delivery WHERE state='uncertain')
  UNION ALL
  SELECT 2,'/retry','Retry delivery',NULL,'button',130,
         json_object('action','command','spec',json_object(
           'type','button','label','Retry delivery','op','control',
           'fixed',json_object('op','retry','job',job)))
  FROM delivery WHERE job_exhausted=0 AND request_exhausted=0
  UNION ALL
  SELECT 3,'/replace','Start a fresh attempt',NULL,'button',166,
         json_object('action','command','spec',json_object(
           'type','button','label','Start a fresh attempt','op','control',
           'fixed',json_object('op','replace','job',job)))
  FROM delivery WHERE request_exhausted=0 AND (job_exhausted=1 OR expanded)
  UNION ALL
  SELECT 4,'/fresh-topic','New topic',NULL,'button',112,
         json('{"action":"nav","patch":{"view":"relay","reply":null,"steer":null,"follow":null,"revise":null,"fresh":true,"older":null,"newer":null}}')
  FROM delivery WHERE request_exhausted=1 AND budget_row=1
)
SELECT CASE WHEN kind='label' THEN
  json_object('id',key||suffix,'kind',kind,'label',label,'role','muted','value',value)
ELSE json_object('id',CASE WHEN suffix='/replace' THEN base||'/replace/'||job ELSE key||suffix END,
  'kind',kind,'label',label,'role','quiet','width',width,'row',key,'event',json(event))
END AS "control:json"
FROM delivery CROSS JOIN choices
ORDER BY n
