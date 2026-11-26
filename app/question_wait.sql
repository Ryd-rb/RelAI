SELECT 1 AS ok
FROM jobs message JOIN requests r ON r.id=message.request AND r.project=message.project
    AND r.revision=message.revision AND r.state='open'
JOIN questions addressed ON addressed.id=message.input->>'$.answer_to'
    AND addressed.request=message.request AND addressed.revision=message.revision
    AND addressed.session=message.session AND addressed.role=message.role AND addressed.state='answer_queued'
JOIN actions original ON original.id=addressed.id AND original.kind='ASK'
JOIN jobs origin ON origin.id=original.job AND origin.project=message.project
    AND origin.request=message.request AND origin.revision=message.revision
    AND origin.session=message.session AND origin.role=message.role
JOIN actions ask ON ask.job=origin.id AND ask.kind='ASK' AND ask.state='admitted'
JOIN questions sibling ON sibling.id=ask.id AND sibling.id!=addressed.id
    AND sibling.request=message.request AND sibling.revision=message.revision
    AND sibling.session=message.session AND sibling.role=message.role
    AND sibling.state IN ('open','answer_queued')
WHERE message.id=?1 AND message.kind='message'
LIMIT 1
