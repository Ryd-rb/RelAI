UPDATE branches AS b
SET accepted=json_object('verifier','relay','evidence',b.report,'revision',b.revision)
WHERE b.id IN (SELECT id FROM ready_branch_reports WHERE request=?1 AND revision=?2)
