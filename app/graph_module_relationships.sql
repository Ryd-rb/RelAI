WITH targets AS (
    SELECT name,min(path) AS path,count(*) AS n
    FROM graph_symbols
    WHERE snapshot=?1 AND language='fluxr' AND role='symbols'
    GROUP BY name
), calls AS (
    SELECT s.path AS source,t.path AS target,s.digest,s.name,s.start,s.end,'call' AS kind
    FROM graph_symbols s JOIN targets t ON t.name=s.name AND t.n=1 AND t.path<>s.path
    WHERE s.snapshot=?1 AND s.language='fluxr' AND s.role='references'
), source_files AS (
    SELECT path,parent,
           substr(path,length(parent)+CASE WHEN parent='' THEN 1 ELSE 2 END) AS filename
    FROM inventory
    WHERE snapshot=?1 AND kind='file' AND path GLOB '*.rs'
), crate_roots AS (
    SELECT DISTINCT r.value AS path
    FROM graph_file_metadata f, json_each(f.metadata,'$.roots') r
    WHERE f.snapshot=?1
), module_directories AS (
    SELECT path,
           CASE WHEN parent='' THEN '' ELSE parent||'/' END ||
           CASE WHEN filename IN ('main.rs','lib.rs','mod.rs')
                     OR path IN (SELECT path FROM crate_roots) THEN ''
                ELSE substr(filename,1,length(filename)-3)||'/' END AS directory
    FROM source_files
), modules AS (
    SELECT s.path AS source,m.path AS target,s.digest,s.name,s.start,s.end,'module' AS kind
    FROM graph_symbols s
    JOIN module_directories d ON d.path=s.path
    JOIN inventory m ON m.snapshot=s.snapshot AND m.kind='file'
      AND m.path IN (d.directory||s.name||'.rs',d.directory||s.name||'/mod.rs')
    WHERE s.snapshot=?1 AND s.language='rust'
      AND s.role='symbols' AND s.kind='external_module'
      AND 1=(SELECT count(*) FROM inventory c WHERE c.snapshot=s.snapshot AND c.kind='file'
             AND c.path IN (d.directory||s.name||'.rs',d.directory||s.name||'/mod.rs'))
), overrides AS (
    SELECT f.path AS source,m.path AS target,f.digest,
           a.value->>'$.name' AS name,a.value->>'$.start' AS start,a.value->>'$.end' AS end,'module' AS kind
    FROM graph_file_metadata f JOIN json_each(f.metadata,'$.modules') a
    JOIN inventory m ON m.snapshot=f.snapshot AND m.kind='file' AND m.path=(a.value->>'$.target')
    WHERE f.snapshot=?1
)
SELECT * FROM (SELECT * FROM calls UNION ALL SELECT * FROM modules UNION ALL SELECT * FROM overrides)
ORDER BY source,start,target LIMIT 513
