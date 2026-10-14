# G00 early-usefulness protocol (approved; criteria frozen)

**Tasks:** P0-D01 (criteria), P0-D03 to P0-D06 in the [execution plan](../execution-plan.md) · **Spec:** PRD v0.3 §12.4, §13.1 · **Consent:** [g00-consent-form.md](g00-consent-form.md)
**Version:** 1.0 · **Drafted:** 14 October 2026 · **Approved as written:** 14 October 2026 · **Status:** **Approved. The criteria in §3, the scoring rubric in §5.3 and the decision rules in §8 are frozen.** The start date, weekly availability, reference host and names in §10 are still to be filled in, and that must happen before day 1. The consent form is a separate document and is not yet approved.

Items in [square brackets] are blanks for you to fill. Items marked **[proposed]** are my choices where the PRD is silent; they are the ones to review hardest. The PRD's own wording is quoted or cited, never altered.

## 1. Purpose and what this does not show

Test whether a vocalist finds three things useful enough to bring back another recording: a
no-MIDI pitch view, a selected-note exercise, and a level-matched gain/EQ/compression preview of
their own recording. Meeting G00 supports investing in the integrated prototype. It is **not**
market validation, and it does not show professional audio quality or demand (PRD §12.4).

The sessions use **copies** of recordings and produce **separate preview files**. No participant's
working session is changed. A developer may prepare outputs by hand; that is allowed and is recorded
(§6), so a prepared demonstration is never reported as automated product performance.

## 2. Schedule (from the recorded start date, day 1 = start)

| Day | Step | Task |
|---|---|---|
| Before day 1 | Record the start date, weekly availability and the reference host details in §10 | Plan, Phase 0 |
| **1** | Approve and sign §3 (criteria frozen). Approve the consent form. Start recruiting | P0-D01, D02, D03 |
| 1–10 | Recruit. **Roster closes on day 10** | P0-D03 |
| 1–10 | Prepare the tooling and the per-participant preparation workflow | P0-D04 |
| 8–**21** | Run the sessions. **Every initial session finishes by day 21**, so every 7-day window closes by day 28 | P0-D05 |
| 28 | Score against §3 and issue the decision | P0-D06 |

A session held after day 21 is still run if it is useful, but the return-use result for that
participant will be unresolved at scoring and is treated under §3.4.

## 3. Go criteria, frozen before any outcome is collected (approved 14 October 2026)

These restate PRD §12.4. The count rules are mine **[proposed]** and are what the PRD means by
"round required participant counts upwards".

### 3.1 Definitions

- **Completed session:** the participant finished the pitch view, the selected-note example, the
  comprehension check (§5.3) and the keep/discard step for their actionable recording. A
  participant who withdraws part-way, or whose recording could not be used, is not completed.
  Their data is kept only as far as the consent form allows and reported as non-completion.
- **n:** the number of completed sessions.
- **Actionable recording:** a recording where the participant and the researcher agree before
  preparation that there is a fixable issue in gain, tone or level evenness (§4.2). One per
  participant. It is the only recording that counts toward the keep criterion.
- **Clean control:** a second recording (or section) where keeping the original is reasonable. It is
  shown but **excluded from the keep criterion [proposed]**, as the PRD excludes it for G11.
- **Keep:** the participant chooses the processed candidate over the original for the actionable
  recording, after level-matched A/B listening. Choosing the original is a valid outcome and counts
  as not keeping.
- **Second recording:** a new recording of the participant's own, sent by them within 7 days of
  the end of their session, that they ask to try again. The 7 days run from the session's end.

### 3.2 The four criteria

| # | Criterion (PRD §12.4) | Numerator | Denominator |
|---|---|---|---|
| C1 | At least 6 completed sessions | n | – |
| C2 | At least two-thirds can explain the difference between measured pitch, nearest degree and intended-note error | Participants who pass the §5.3 rubric | n |
| C3 | At least two-thirds keep a candidate on the actionable recording | Participants who keep | n |
| C4 | At least half provide a second recording within seven days | Participants who send one in their window | **m**: completed participants whose 7-day window has closed at scoring |

### 3.3 Required counts, rounded up

| n or m | Two-thirds (C2, C3) | Half (C4) |
|---|---|---|
| 6 | 4 | 3 |
| 7 | 5 | 4 |
| 8 | 6 | 4 |

### 3.4 Participants whose window is still open at scoring

A participant whose 7-day window has not closed on the scoring date is **excluded from m**. They are
reported, and counted neither as returning nor as not returning. If m is less than 6, C4 is
**unresolved** (it is not "met" and not "failed"). Day 21 / day 28 in §2 exists so that normally
m = n.

## 4. Participants and materials

### 4.1 Recruitment (6 to 8 completed sessions)

Independent vocalists recording at home. The roster must include **at least two low-register
singers [proposed]** (the PRD requires "low-register singers" without a number) and **at least two
participants who have no MIDI melody for their song**. Overlap counts. Recruit up to 8 so that a
withdrawal still leaves 6. Select by stated register and by whether they use MIDI, not by gender.
Record the register as self-described and, where possible, from the measured pitch range.

Exclusions **[proposed]**: under 18; recordings they cannot confirm they have permission to share
(§4.3); anyone who cannot give informed consent in English (the PRD limits the first release to
English).

### 4.2 The three things every participant sees

1. **No-MIDI pitch view** of their own recording: the continuous pitch contour relative to a Sa they
   confirm, voiced regions, and uncertainty, with an optional nearest-degree label that does not snap
   the contour.
2. **Selected-note example:** they choose a target swara, octave and a sustained region; they see
   cents deviation, stability and confidence for that region only.
3. **Level-matched gain/EQ/compression preview** for one agreed issue on the actionable recording,
   plus a clean control. A/B with loudness matched (§5.2), A and B order randomised.

### 4.3 Recordings

Only recordings the participant has permission to share (their own performance and, if the song is
someone else's, material they are allowed to share for this purpose; recommended: original songs or
public-domain material). Recordings are copied. Originals are never opened for writing.

## 5. Session design (about 60 minutes)

### 5.1 Running order

| Min | Step | Recorded |
|---|---|---|
| 0–5 | Consent check; confirm the permitted-use choices; start the manifest (App. A) | Consent and choices |
| 5–10 | Confirm Sa and octave; confirm the actionable issue and the clean control | Issue in the participant's words |
| 10–25 | Pitch view of their recording; they explore it | Observations, questions, help given |
| 25–35 | Selected-note example | Target chosen, observations |
| 35–40 | **Comprehension check** (§5.3), before any explanation beyond the on-screen labels | Rubric result |
| 40–55 | A/B preview of the actionable recording and the control, loudness matched; keep or discard each | Keep, reason, order shown |
| 55–60 | Close: what they would change; explain the 7-day follow-up and the reminder rule (§5.4) | Comments |

### 5.2 Level matching

The two versions are matched to the same integrated loudness before comparison, using a **method
named and recorded in the manifest** (for example ITU-R BS.1770 through a stated tool and
version) and matched within 0.5 LU. If a reliable match isn't possible (very short or near-silent
material), say so and match by ear with the participant, and record that. Matching applies to
listening only. No loudness-matched file is given to the participant as an export unless they ask
for the unmatched preview file.

### 5.3 Comprehension check and rubric (for C2)

Asked after the participant has used the pitch view and the selected-note example, with no coaching
beyond the on-screen labels. Three questions, in the participant's own words:

| Q | Question | Pass requires saying that |
|---|---|---|
| Q1 | "What does the line on the screen show?" | It is the pitch actually measured from their voice, over time (relative to Sa) |
| Q2 | "What does the note name or label next to it mean?" | It is the scale note the measured pitch is **nearest to**: proximity, not a verdict, and not where they were aiming |
| Q3 | "Does the app tell you whether you sang the note you meant to?" | No, not unless they pick a target note; with a target it shows how far they were from **that** target |

A participant **passes C2 only if all three pass**. Two of three is recorded as "partial" and does not
count. Scoring is by two people **[proposed]**: the researcher at the time, and a second reviewer
from the notes or recording, who doesn't see the first score. Disagreements are listed and resolved
by a third look at the notes before the decision, and the original scores are kept in the record.

### 5.4 The 7-day follow-up

At the end of the session the participant is told they may send a second recording within 7 days if
they want another trial; there is no obligation. **One neutral reminder on day 5 [proposed]**
("The window for a second recording closes on <date>"), identical for everyone and logged; no
other prompting and no incentive tied to returning. A second recording sent on day 7 counts; one
sent after the window does not count toward C4 but is recorded.

## 6. Logging manual work separately

Per participant, in the manifest (App. A), as three separate counters, each with minutes and a
description:

1. **Manual preparation** before the session (cleaning, picking sections, choosing processor
   settings by hand, rendering previews).
2. **Researcher assistance** during the session (anything done for the participant, hints, explanations
   beyond the screen labels).
3. **Session time** (participant's actual time using each part).

Which previews came from the headroom rule (`level.headroom` 0.1.0, [production-policy.md](../production-policy.md))
and which were made by hand is recorded per preview. EQ and compression previews are hand-prepared at
this stage. State this in any summary of results.

## 7. Data handling and consent

Governed by [g00-consent-form.md](g00-consent-form.md). Summary of the rules the protocol relies on:

- Recordings stay on [named device(s)], are not uploaded to any cloud service or sent to any AI
  model, and are not used for model training. Training would need a **separate**, later consent.
- Participants are identified by a code. The code-to-person key is kept apart from the recordings.
- Deletion: on request at any time, and in any case at [retention period, for example 90 days after
  the decision]. Deletion is confirmed in writing to the participant.
- After deletion of the audio, two things remain. The **manifest** (App. A) holds no audio and no
  name (a code, dates, choices and counts). The **signed consent form** does hold the participant's
  name, so it is filed apart from everything else and kept only for [period, for example 12 months]
  as proof of consent, then destroyed. Contact details are deleted with the recordings.

## 8. What gets reported, and the decision rules

### 8.1 Always reported, separately

- n, m, and who is excluded from m and why.
- C2, C3, C4 as counts, against the required counts in §3.3.
- **Pitch assistance and production assistance reported separately**: C2 covers pitch; C3 covers
  production. Add one line each on the participants' stated value for each (what they would use again).
- Ties and rejection reasons, in the participants' words, per preview type (gain, EQ, compression).
- The three logged manual-work counters (§6).
- Anything that went wrong in a session.

### 8.2 Decision **[proposed]**

| Result | Decision |
|---|---|
| n < 6 | **Inconclusive** (PRD) |
| n ≥ 6 and C2, C3 and C4 all met | **Go**: supports investing in the integrated prototype |
| n ≥ 6, C2 and C3 met, C4 unresolved because m < 6 | **Inconclusive** on return use. Not Go. State what is needed (more participants or more days) |
| n ≥ 6 and at least one of C2, C3, C4 met and at least one not met | **Mixed**: a named, bounded redesign or a proposed scope change. **Never averaged into success** (PRD) |
| n ≥ 6 and none of C2, C3, C4 met | **Stop and reassess** the product promise |

**Special case (PRD):** if pitch adds no useful value but production does (C2 not met, C3 met), write
that finding down and amend the scope explicitly **before** proceeding with a different product
promise. The decision is then Mixed, with the amendment as the named change.

The decision memo is dated and issued on day 28. It does not extend the study silently.

## 9. Risks to the result

- A hand-prepared demonstration can look better than the product will be; §6 and §8.1 are there to
  keep that visible.
- 6 to 8 people is small. The criteria are thresholds for a go/no-go on investment, not statistics.
- Participants recruited through the researcher's network may be kinder than strangers. Record how each
  was recruited.
- Choosing the original is a valid result, and a clean control exists so that "I'd keep the original"
  isn't read as a failure of the participant.

## 10. Approval and start record (fill in before day 1)

| Field | Value |
|---|---|
| Start date (day 1) | [date] |
| Weekly availability (hours) | [hours per week; the plan caps Phase 0 at 40 engineering hours in total] |
| Reference host | [Apple M1, 16 GB, macOS build, free disk] |
| Researcher(s) and second scorer | [names or codes] |
| Approver | [name to be entered; approval was given in the working session on 14 October 2026] |
| Criteria in §3 frozen, with the rules in §3.3, §3.4 and §8.2 | ☒ approved as written, 14 October 2026 ☐ approved with these changes |
| Signature and date | [signature to be added]; approval recorded 14 October 2026 |

### Change control for the frozen sections

§3 (criteria and count rules), §5.3 (the understanding rubric), §3.4 and §8 (decision rules) may not change
after the first outcome is collected. Before that, a change needs a new protocol version and a new dated
approval, recorded in the revision history below. Any change after outcomes exist is a new, separate
experiment and is reported as such; it never rescues a result. Anything else (wording, logistics, the
consent form) can change with a note in the revision history.

## Appendix A: per-participant manifest (also the PRD §7.1 input/output manifest)

| Field | Value |
|---|---|
| Participant code | [P01…P08] |
| Recruited by / how | [ ] |
| Self-described register; measured range (Hz) | [ ] |
| Uses MIDI for this song? | yes / no |
| Consent form version and date signed | [ ] |
| Permitted-use choices (consent §5) | [A] [B] [C] |
| Retention and deletion date | [ ] |
| Input files: name, source, SHA-256, duration, sample rate | [ ] |
| Sa and octave confirmed | [ ] |
| Actionable issue (participant's words) and control | [ ] |
| Output files (each preview): name, SHA-256, how made (rule `level.headroom` 0.1.0 / by hand), settings, loudness method and match (LU) | [ ] |
| Manual preparation: minutes and description | [ ] |
| Researcher assistance: minutes and description | [ ] |
| Session time: minutes per step | [ ] |
| A/B order shown | [ ] |
| Q1, Q2, Q3 result and scorer 1 / scorer 2 | [ ] |
| Keep (actionable): keep / original; reason | [ ] |
| Control: keep / original; reason | [ ] |
| Pitch value in their words; production value in their words | [ ] |
| Session end date; window closes (end + 7 days); reminder sent (date) | [ ] |
| Second recording received (date) / not received / window open at scoring | [ ] |
| Deletion confirmed (date) | [ ] |

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Draft protocol for P0-D01 to D06. Not approved. |
| 1.0 | 14 October 2026 | Approved as written. Criteria, rubric and decision rules frozen. Blanks in §10 remain. |
