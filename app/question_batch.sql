-- Only admitted questions from the addressed question's original envelope.
SELECT sibling.id AS question_id,sibling.body->>'$.question' AS question,sibling.state,
    (SELECT answer.payload->>'$.payload'
       FROM actions answer JOIN jobs intake ON intake.id=answer.job
       JOIN projects p ON p.id=intake.project AND p.discriminator=intake.session
       JOIN submissions submission ON submission.id=intake.input->>'$.submission'
          AND submission.project=intake.project AND submission.request=intake.request
      WHERE answer.kind='MESSAGE' AND answer.state='admitted'
        AND answer.payload->>'$.question'=sibling.id
        AND answer.payload->>'$.destination'=sibling.session
        AND answer.payload->>'$.revision'=sibling.revision
        AND intake.project=?2 AND intake.request=?3 AND intake.revision=?4
        AND intake.kind='intake' AND intake.role='discriminator' AND intake.state='accepted'
        AND intake.input->>'$.reply'=sibling.id
        AND instr(submission.content->>'$.text',answer.payload->>'$.payload')>0
        AND sibling.state IN ('answer_queued','answered')
      ORDER BY answer.rowid DESC LIMIT 1) AS answer
FROM questions addressed JOIN actions original ON original.id=addressed.id AND original.kind='ASK'
JOIN jobs origin ON origin.id=original.job AND origin.project=?2
    AND origin.request=?3 AND origin.revision=?4 AND origin.session=?5 AND origin.role=?6
JOIN actions ask ON ask.job=origin.id AND ask.kind='ASK' AND ask.state='admitted'
JOIN questions sibling ON sibling.id=ask.id AND sibling.request=?3 AND sibling.revision=?4
    AND sibling.session=?5 AND sibling.role=?6
WHERE addressed.id=?1 AND addressed.request=?3 AND addressed.revision=?4
    AND addressed.session=?5 AND addressed.role=?6
ORDER BY ask.rowid
