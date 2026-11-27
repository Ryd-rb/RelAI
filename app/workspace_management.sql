SELECT
  (SELECT json_group_array(json_object('value',id,'label',name||' · '||
      CASE WHEN owner=?1 THEN 'Owned here' ELSE 'Owned by '||coalesce((SELECT value->>'$.short' FROM json_each(?2) WHERE value->>'$.id'=d.owner),owner,'unknown') END||
      CASE WHEN enabled THEN ' · Linked' ELSE ' · Available' END))
   FROM (SELECT d.*,EXISTS(SELECT 1 FROM dictionary_links l WHERE l.project=?1 AND l.dictionary=d.id AND l.enabled=1) AS enabled
         FROM dictionaries d WHERE archived=0 AND id<>'RelAI' ORDER BY owner=?1 DESC,name,id) d) AS "available:json",
  (SELECT json_group_array(json_object('value',id,'label',name)) FROM
      (SELECT id,name FROM dictionaries WHERE owner=?1 AND archived=0 ORDER BY name,id)) AS "owned:json",
  (SELECT json_group_array(json_object('value',id,'label',name||' · '||digest)) FROM
      (SELECT id,name,digest FROM skills WHERE project=?1 AND active=1 ORDER BY name,digest,id)) AS "installed:json"
