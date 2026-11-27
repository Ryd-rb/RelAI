# Open app issues

- [ ] 003. In a maximized 1280-wide desktop window, the composer fold reads “Other questi…” even with ample room alongside it. Show its full label.
- [ ] 004. Click Accept answer in Relay. The button disappears and keyboard focus lands on an unnamed pane. Focus the follow-up composer so typing can continue.
- [ ] 005. Select fs.read in the Tools library. The Arguments editor opens empty, with no indication that the user should supply a JSON object with a path. Show the selected tool's input contract beside its editor.
- [ ] 006. Run fs.read with empty arguments in Tools. It reports “read_base64: Expected text”. Identify the missing file path in terms the user can act on.
- [ ] 007. Ask Relay to inspect many parts of the app. The model batches more than eight actions, the harness rejects the response, and pressing Retry delivery can hit the same limit again. Expose the eight-action bound to the model before it plans a batch.
- [ ] 008. After a delivered response exceeds the action limit, chat says “Delivery could not be confirmed”. Distinguish a received but rejected response from a genuinely interrupted delivery. Keep the duplicate-send warning for genuinely uncertain transport.
- [ ] 009. Answer a question, then reread it in chat history. Its suggested answers have disappeared from the record. Keep them readable without offering active reply buttons.
- [ ] 010. Two failed attempts in the Activity timeline both read “Job held”. Each event already has an error field, but only opening the detail distinguishes the cause. Include the concise recorded cause in the row.
- [ ] 011. Search Tools for tool.find. The built-in catalog and the extra services catalog both register it. Return one runnable entry for that built-in.
- [ ] 012. Select fs.read, enter its arguments, then select project.info. The runner still contains the previous tool's JSON. Restore each tool's own arguments when switching between them.
- [ ] 013. Run project.info and then select fs.read. The previous tool's output remains under the new selection. Associate a displayed result with the tool and version that produced it.
- [ ] 014. Search for a registered tool with two versions of the same contract. Both matching rows have the same visible name and description. Identify each exact version before the user selects it.
- [ ] 015. Enter a metadata-only term such as “portable” in the Tools library. It matches hidden routing metadata despite the search box promising a name or description search. Make the desktop search agree with its label and preserve full-record search for tool.find callers.
