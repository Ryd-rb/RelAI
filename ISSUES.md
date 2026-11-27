# Open app issues

- [ ] 008. After a delivered response exceeds the action limit, chat says “Delivery could not be confirmed”. Distinguish a received but rejected response from a genuinely interrupted delivery. Keep the duplicate-send warning for genuinely uncertain transport.
- [ ] 028. Create “Café”, then type “CAFÉ” in term completion or Dictionary search. The match disappears because SQL lower only folds ASCII. Use Unicode case folding consistently for term-name matching.
- [ ] 031. Choose an unqualified version in the Tools library. It looks runnable and the Run button is enabled although the backend refuses to execute it. Identify its lifecycle state and disable dispatch when it cannot run.
- [ ] 032. Save a Models preset with a spaces-only model ID. It persists and will be used for the next model call. Reject whitespace-only IDs and keep the editor open.
- [ ] 033. Clear Model ID, then choose to restore the inherited preset. The form requires the deleted ID even though reset needs only the role and scope. Allow reset without an ID.
- [ ] 034. Click Edit assignment on Models. Focus lands on the unnamed pane (observed in the native window). Put it in the configuration form.
- [ ] 035. Fail a Tools operation and then open Versions. The old error follows the user onto that tab and there is no Dismiss control outside chat. Allow dismissal on workspace pages and clear obsolete feedback on tab changes.
- [ ] 036. Unlink the project's own dictionary, then use Save definition on Terms. The save reports success but the term cannot be found in this project's search or completion. Explain that the destination dictionary is unlinked before saving.
- [ ] 037. Search the Activity timeline for when an event occurred. The record stores only a sequence number and the visible rows show no time. Timestamp new events and preserve readable legacy records with unknown times.
- [ ] 038. Open Needs attention after a held job has been superseded or a question answered. The historical event remains in the actionable list solely because of its original event kind. Keep history in Timeline and filter Needs attention by current state.
- [ ] 039. Reach maximum or minimum zoom in Project graph. The corresponding Zoom button remains enabled and repeated presses silently do nothing. Reflect the bounds in the toolbar.
- [ ] 040. Press Fit in Project graph. It sets zoom to 100 percent regardless of content or viewport. Label the action as a zoom reset and indicate the current scale.
- [ ] 041. Start RelAI with a preconfigured CARGO_TARGET_DIR. The launcher builds into that directory but tries to launch target/release from the repository, which can start an old binary or fail. Launch the executable produced by that build.
