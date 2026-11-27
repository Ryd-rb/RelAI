WITH RECURSIVE lines(n,rest,preview) AS (
  SELECT 0,substr(?1,1,1200),''
  UNION ALL SELECT n+1,
    CASE WHEN instr(rest,char(10))>0 THEN substr(rest,instr(rest,char(10))+1) ELSE '' END,
    preview||CASE WHEN instr(rest,char(10))>0 THEN substr(rest,1,instr(rest,char(10))) ELSE rest END
  FROM lines WHERE n<8 AND rest<>''
)
SELECT rtrim(preview,char(10)||char(13))||char(10)||'…' AS body FROM lines ORDER BY n DESC LIMIT 1
