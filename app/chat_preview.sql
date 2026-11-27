-- Scan one extra character so a separator at the limit does not drop a word.
WITH RECURSIVE input(head, ws) AS (
    SELECT substr(?1, 1, 1201), char(9, 10, 11, 12, 13, 32)
), scan(pos, lf_count, last_lf, last_ws) AS (
    SELECT 0, 0, 0, 0
    UNION ALL
    SELECT pos + 1, lf_count + (substr(head, pos + 1, 1) = char(10)),
        CASE WHEN substr(head, pos + 1, 1) = char(10) THEN pos + 1 ELSE last_lf END,
        CASE WHEN instr(ws, substr(head, pos + 1, 1)) > 0 THEN pos + 1 ELSE last_ws END
    FROM scan CROSS JOIN input
    WHERE pos < min(1200, length(head)) AND lf_count < 8
), boundary AS (
    SELECT head, ws, pos, last_lf,
        CASE WHEN pos < length(head) AND instr(ws, substr(head, pos + 1, 1)) > 0
             THEN pos ELSE last_ws END AS space_cut
    FROM scan CROSS JOIN input
    ORDER BY pos DESC LIMIT 1
)
SELECT CASE WHEN pos = length(head) THEN head ELSE coalesce(
    nullif(rtrim(substr(head, 1, last_lf), ws), ''),
    nullif(rtrim(substr(head, 1, space_cut), ws), ''),
    rtrim(substr(head, 1, pos), ws)
) || '…' END AS body
FROM boundary
