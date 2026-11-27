# Open app issues

- [ ] 008. After a delivered response exceeds the action limit, chat says “Delivery could not be confirmed”. Distinguish a received but rejected response from a genuinely interrupted delivery. Keep the duplicate-send warning for genuinely uncertain transport.
- [ ] 028. Create “Café”, then type “CAFÉ” in term completion or Dictionary search. The match disappears because SQL lower only folds ASCII. Use Unicode case folding consistently for term-name matching.
- [ ] 037. Search the Activity timeline for when an event occurred. The record stores only a sequence number and the visible rows show no time. Timestamp new events and preserve readable legacy records with unknown times.
- [ ] 038. Open Needs attention after a held job has been superseded or a question answered. The historical event remains in the actionable list solely because of its original event kind. Keep history in Timeline and filter Needs attention by current state.
- [ ] 070. Produce over 100 events, then open Activity. Timeline silently discards all but the latest 100 with no way back. Provide bounded browsing to older records with a stable cursor.
- [ ] 076. Capture a Rust project using `mod foo;` with `foo/mod.rs`, or a nested `mod child;` in `foo.rs`. Relationships misses these valid module locations. Resolve both conventional layouts with exact snapshot paths.
- [ ] 077. Capture a .rs or .fluxr file containing invalid UTF-8. Graph coverage counts it as indexed even though the indexer skipped it. Report only files actually examined and retain the total candidate count.
