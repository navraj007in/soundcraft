# UI decisions

Decisions that change or extend the received design. [ui-design-spec.md](ui-design-spec.md) and [copy.json](copy.json) stay a faithful record of the bundle; where a decision overrides the bundle, it is listed here, and this document wins.

## UI-1 Approval flow

**Decided:** 14 October 2026, on your instruction to adopt the recommended flow. **Status:** decided, not yet implemented.

### The problem

The design's change review offers **Preview · Approve · Reject**, and its audition then offers **Keep this change · Keep the original · Discard**. A user could approve a change and then be asked to keep it, which is two approvals for one change, and an approval given before hearing the candidate.

### What the PRD says (v0.3, §5.2 and §5.1)

- "Apply: execute a reviewed, authorised plan after state preconditions pass. **For production proposals, Keep in the audition view is the approval action. Simple supported edits can be approved from the change list without a render.**"
- "Editing any operation invalidates the previous candidate and approval. A reference or session change affecting the result marks it stale. No mode silently upgrades itself to Apply."
- §5.1 step 9: "Choose Keep to approve **that exact candidate** for the working session, or Discard."
- §7: an approval is bound to "the exact reviewed operation set and candidate, not only the latest chat message."

### The decision

There are two approval paths, and the **controller chooses the path, never the model or the UI**. A plan carries `approval_path`, set from where the plan came from:

| | **Production proposal** (`audition`) | **Explicit assignment** (`change_list`) |
|---|---|---|
| Comes from | A local goal rule: headroom now; clarity (EQ) and even level (compression) later | The user stating an exact value or name, for example "set Kick to −6 dB" or "rename this track" (the qualified assignments in FR-02) |
| Change review shows | The proposal, its basis, limits and expected effect (board 3c) | The change, its before and after values and its target (board 3c) |
| Change review actions | **Preview** (primary) and **Reject**. **There is no Approve.** | **Approve** (primary) and **Reject**. No render |
| Preview | Renders the candidate (task state Previewing, then Ready for approval) and opens the audition (board 3d) | Not offered |
| Approval action | **Keep this change** in the audition | **Approve** in the change list |
| The approval binds | Plan digest **and the candidate digest** and the expected content revision | Plan digest (the operation set) and the expected content revision |
| Declining | **Keep the original** or **Discard** in the audition, or **Reject** in the change review: all end in Rejected, session untouched | **Reject**: Rejected, session untouched |

Things that hold on both paths:

- **One approval per change.** Nothing before it approves anything.
- **Keep this change, Keep the original and Discard stay equal in size and weight.** Initial keyboard focus in the audition is the A/B switch, not any of the three.
- **Editing any value in the change review invalidates the candidate and any approval.** On the audition path Preview becomes available again and Keep is off with the stale message (the design already has it). The audition's Keep is also off when the session changed after the proposal.
- **A stale plan cannot be approved on either path.**
- **No Approve exists for a production proposal, ever**, so a conversation or a chat message cannot approve a candidate that was never heard. Chat text is not an approval on either path.
- **Declining records why.** Rejected carries a reason code: `rejected_before_preview`, `kept_original`, `discarded` (and `rejected` for an explicit assignment). It does not change the session, and it is what the G00 and G11 counts of "kept the candidate" versus "kept the original" are read from.

### Changes to the design's strings

`copy.json` keeps the original wording. Implement these instead:

| Where | Design says | Implement |
|---|---|---|
| Change review actions, production proposal | `Preview · Approve · Reject` | `Preview · Reject` |
| Change review actions, explicit assignment | (not designed) | `Approve · Reject` |
| Change review interaction note | "Editing any value turns the preview stale and enables Render. Approve is off while the preview is out of date. Reject needs no confirmation." | "Editing any value turns the preview stale. Preview renders it again and opens the audition. Reject needs no confirmation. A proposal is approved in the audition, not here." |
| Change review, disabled state | "Approve is off with the message “Render the preview first” when the preview is stale." | Production proposal: Preview is off with the reason beside it when no preview can be made, for example "A plug-in in the signal path cannot be rendered here." Explicit assignment: Approve is off while the plan is stale, with the message "The session changed. Ask again to review this change." |
| Task history, Rejected | "You rejected this proposal. Nothing changed." | Keep for Reject and Discard. For Keep the original: "You kept the original. Nothing changed." |

The two strings marked as new ("A plug-in in the signal path cannot be rendered here.", "The session changed. Ask again to review this change.", "You kept the original. Nothing changed.") are **proposed** wording in the design's style (plain, no jargon, says nothing was changed). Have them confirmed or redrawn by the design tool along with the other missing messages (spec §9 item 3).

### Effects on the plan

| Task | Effect |
|---|---|
| P1-C01 | The Plan record carries `approval_path` (`audition` or `change_list`), set by the controller. Validation rejects an approval whose path does not match the plan |
| P1-C04 | An approval is created only by Keep (audition path) or Approve (change-list path), and binds the digests above; nothing else creates one |
| P1-I03 | Actions per path as above; no Approve on a production proposal |
| P1-I04 | Keep this change is the approval action; the three actions are equal; stale handling as designed |
| P1-I05 | Rejected shows its reason; "Ready for approval" on the audition path means "the audition is open" |

### Acceptance tests

1. A production proposal's change review has no Approve control, at any size and in any state (a UI test on the rendered tree).
2. Keep this change creates an approval bound to the plan digest, the candidate digest and the expected content revision; changing any of the three before the click makes the click fail with the stale message.
3. Approve on an explicit assignment creates an approval bound to the operation set and the content revision, and applies without a render.
4. An approval whose `approval_path` does not match its plan is refused by the controller (a production proposal cannot be approved from the change list, an assignment cannot be approved from an audition).
5. Keep the original, Discard and Reject each end in Rejected with the right reason and leave the session byte-identical.
6. Editing a value after Preview invalidates the candidate and the approval; Keep is off until a new preview is rendered.
7. A plan made stale by a session change cannot be approved on either path.
8. A chat message such as "yes, apply it" does not approve either path.

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | UI-1 approval flow decided. |
