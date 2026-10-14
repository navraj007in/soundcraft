# UI design specification (digest of the Claude Design bundle)

**Version:** 0.1 · **Date:** 14 October 2026 · **Status:** Design received and reviewed; **not yet implemented.**
**Source:** the Claude Design bundle "Arrange Window Design Round 1" (rounds 1 to 5), made from [ui-design-brief.md](../ui-design-brief.md).
**Decisions that override the bundle:** [decisions.md](decisions.md).
**Machine-readable:** [tokens.json](tokens.json) (colours, type, sizes, palette, redlines) and [copy.json](copy.json) (interaction notes and exact wording). Both are generated from the bundle as data by `tools/design/extract_design.py`; nothing in the bundle is executed.

## 1. What this is, and what is not in the repository

The bundle is a set of design-canvas pages (`*.dc.html`) plus a third-party runtime script (`support.js`) that draws them. **Neither the zip nor `support.js` is committed**: the runtime has no licence header, and the pages are an authoring format, not something the app consumes. This document and the two JSON files carry what an implementer needs. To regenerate them, unzip the bundle into a new, empty folder and run `python3 -I tools/design/extract_design.py BUNDLE_DIR docs/soundcraft-ai/design`.

**Direction chosen:** the dark **"Graphite"** direction (dense, teal accent), with **IBM Plex Sans** for the interface and **IBM Plex Mono** for numbers, plus a full light theme. The alternative from round 1 ("Paper": warm light, vermilion accent, Figtree and JetBrains Mono) was not carried forward.

**What was and was not checked.** I read the bundle's data and markup as text and recomputed every contrast ratio and the clip palette myself (§3); all 104 interaction and copy strings were checked verbatim against the source. **I did not render the boards**, so the visual layout is described from the markup and the board titles, not seen. Fidelity will be checked by rendering the built app with `ui_shot` next to the boards.

## 2. Boards

| Board | Content | Size |
|---|---|---|
| Round 1: 1a, 1b | Two directions for the Arrange window | 1440 × 900 |
| Arrange Pro, Arrange Pro Light | The chosen direction, dark and light | 1440 × 900 |
| Design System | Colour, type, sizing, motion, clip palette, component states | n/a |
| 3a Vocal inspector | Selected-note practice active | 1440 × 900 |
| 3b Assistant panel | Suggest, running, unavailable | 360 wide (min 320, max 480) |
| 3c Change review | Three result kinds | 440 wide (min 400) |
| 3d Audition | A/B, matching, keep, keep original, discard | 1040 × 640 (min 800 × 520) |
| 3e Task history | All task states | 1040 wide (panel min 640) |
| 3f Recording health | Status bar 40 px, always visible; warning, error, recovery | n/a |
| 3g First run, cloud opt-in, missing media | | n/a |
| 4a Vocalist workspace | One record button, large waveform, pitch view | 1440 × 900 |
| 4b Redlines | Track header and clip at 3×, transport at 2× | logical px |
| 4c States | Fader, pan, automation, track types | n/a |
| 5a Arrange, light | | 1440 × 900 |
| 5b Adapting | 1280 × 800 and 2560 × 1440 (scaled schematics) | n/a |
| 5c Track types | Folder (open), instrument, hidden, aux; header 232 × 74, compact 232 × 40 | n/a |
| Notes and Copy | Interaction notes and microcopy | n/a |

Round 3 screens exist in dark and light.

## 3. Foundations

All values below are generated from the bundle. **Recomputed contrast check against the design's own minimums (text 4.5 : 1; accent, status and graphics 3 : 1; `text.disabled` exempt): no failures in either theme or in the clip palette.**

### 3.1 Colour: dark theme (default; contrast on `bg.surface.1` `#23272c`)

| Token | Value | Usage | Contrast |
|---|---|---|---|
| `bg.canvas` | `#1b1e22` | Arrange canvas, lanes |  |
| `bg.surface.1` | `#23272c` | Panels, inspector, lists |  |
| `bg.surface.2` | `#2e3338` | Raised controls, menus, dialogs |  |
| `bg.surface.3` | `#3a4047` | Selected rows, pressed |  |
| `bg.inset` | `#0a0c0e` | Counter, fields, meter wells |  |
| `bg.chrome.top` | `#383d44` | Toolbar gradient start |  |
| `bg.chrome.bottom` | `#2a2e34` | Toolbar gradient end |  |
| `border.subtle` | `#0e1012` | Dividers, panel edges |  |
| `border.control` | `#6b7683` | Outline of inputs on focus-less state | 3.25:1 on `#23272c` |
| `border.highlight` | `#ffffff12` | Top edge, 1 px inset |  |
| `text.primary` | `#e6e9ed` | Body, values | 12.33:1 on `#23272c` |
| `text.secondary` | `#a9b1ba` | Labels, captions | 6.93:1 on `#23272c` |
| `text.disabled` | `#6b747d` | Disabled (exempt) | 3.16:1 on `#23272c` |
| `accent.default` | `#3ec7b8` | Selection, focus ring, links | 7.2:1 on `#23272c` |
| `accent.on` | `#08201d` | Text on accent fill | 8.15:1 on `#3ec7b8` |
| `status.info` | `#6aa5ff` | Info | 6.04:1 on `#23272c` |
| `status.success` | `#58d67d` | Success | 8.1:1 on `#23272c` |
| `status.warning` | `#f2c14e` | Warning | 8.95:1 on `#23272c` |
| `status.error` | `#ff6b6b` | Error | 5.41:1 on `#23272c` |
| `transport.play` | `#58d67d` | Play active | 8.1:1 on `#23272c` |
| `transport.record` | `#ff5d5d` | Record armed, recording | 4.99:1 on `#23272c` |
| `track.solo` | `#f2c14e` | Solo | 8.95:1 on `#23272c` |
| `track.mute` | `#5b9bff` | Mute | 5.42:1 on `#23272c` |
| `meter.fill` | `#3ec7b8` | Meter 0 to −12 dB | 7.2:1 on `#23272c` |

### 3.2 Colour: light theme (contrast on `bg.surface.1` `#f2f4f6`)

| Token | Value | Usage | Contrast |
|---|---|---|---|
| `bg.canvas` | `#e4e8ec` | Arrange canvas, lanes |  |
| `bg.surface.1` | `#f2f4f6` | Panels, inspector, lists |  |
| `bg.surface.2` | `#ffffff` | Raised controls, menus, dialogs |  |
| `bg.surface.3` | `#dde2e7` | Selected rows, pressed |  |
| `bg.inset` | `#ffffff` | Fields, meter wells |  |
| `bg.chrome.top` | `#fbfcfd` | Toolbar gradient start |  |
| `bg.chrome.bottom` | `#e3e7eb` | Toolbar gradient end |  |
| `border.subtle` | `#c9d0d7` | Dividers, panel edges |  |
| `border.control` | `#76818d` | Outline of inputs | 3.6:1 on `#f2f4f6` |
| `border.highlight` | `#ffffffcc` | Top edge, 1 px inset |  |
| `text.primary` | `#14181c` | Body, values | 16.18:1 on `#f2f4f6` |
| `text.secondary` | `#47525d` | Labels, captions | 7.23:1 on `#f2f4f6` |
| `text.disabled` | `#8a949e` | Disabled (exempt) | 2.8:1 on `#f2f4f6` |
| `accent.default` | `#0b6f66` | Selection, focus ring, links | 5.47:1 on `#f2f4f6` |
| `accent.on` | `#ffffff` | Text on accent fill | 6.03:1 on `#0b6f66` |
| `status.info` | `#1f5fbf` | Info | 5.52:1 on `#f2f4f6` |
| `status.success` | `#16703a` | Success | 5.58:1 on `#f2f4f6` |
| `status.warning` | `#855400` | Warning | 5.83:1 on `#f2f4f6` |
| `status.error` | `#c0262d` | Error | 5.36:1 on `#f2f4f6` |
| `transport.play` | `#16703a` | Play active | 5.58:1 on `#f2f4f6` |
| `transport.record` | `#c0262d` | Record armed, recording | 5.36:1 on `#f2f4f6` |
| `track.solo` | `#855400` | Solo | 5.83:1 on `#f2f4f6` |
| `track.mute` | `#1f5fbf` | Mute | 5.52:1 on `#f2f4f6` |
| `meter.fill` | `#0b7a70` | Meter 0 to −12 dB | 4.72:1 on `#f2f4f6` |

State is never carried by colour alone: every state also has a label, shape or icon (for example "on" adds a check, "mixed" a dash and a dashed border, "implicitly muted" a dashed outline).

### 3.3 Typography

| Style | Size / line | Weight | Family | Sample |
|---|---|---|---|---|
| `window.title` | 20 / 28 | 600 | IBM Plex Sans | Arrange |
| `panel.heading` | 14 / 20 | 600 | IBM Plex Sans | Inspector |
| `label` | 12 / 16 | 500 | IBM Plex Sans | Input · Interface 1 |
| `body` | 13 / 20 | 400 | IBM Plex Sans | Track names, menus, fields |
| `caption` | 11 / 16 | 400 | IBM Plex Sans | Grid 1/16 · Nudge 1/32 |
| `overline` | 10 / 12 | 500 | IBM Plex Sans | EDIT MODE |
| `number` | 12 / 16 | 400 | IBM Plex Mono (tabular) | −3.0 dB   96.0 BPM |
| `counter.bars` | 24 / 28 | 500 | IBM Plex Mono (tabular) | 005 | 02 | 314 |
| `counter.time` | 16 / 20 | 400 | IBM Plex Mono (tabular) | 0:09.428 |

Fonts: IBM Plex Sans (weights 400, 500, 600) and IBM Plex Mono (weights 400, 500), which the design states are SIL OFL; those weights are the ones its pages request, so those are the files to bundle. **Before bundling, take the licence text from IBM's own repository and add `ATTRIBUTION.md` rows** (`cargo xtask assets` fails the build otherwise). Do not rename or modify the fonts.

### 3.4 Sizing, shape and motion

| Item | Value | Note |
|---|---|---|
| space | 4 8 12 16 20 24 32 48 | base unit 4 px |
| menu bar | 28 | px |
| toolbar | 64 | buttons 28 / 40, counter 48 |
| ruler | 44 | bars 28 + markers 16; collapsed 28 |
| track header | 74 | compact 40 |
| panel header | 28 | px |
| bottom panel tabs | 32 | tab 26, status line 24 |
| button small / normal | 28 / 36 | px |
| transport button | 40 × 40 | play 52 × 40 |
| track button | 22 × 18 | in header; 28 × 22 in inspector |
| text field / dropdown | 24 / 28 | inspector 24, toolbar 28 |
| min click target | 22 × 18 | dense controls; 28 elsewhere |
| panel widths | 232 · 200 | inspector min 200 max 320; right list min 160 max 320 |
| bottom panel | 128 | min 32 (tabs only) max 400 |
| radius | 4 / 4 / 6 | control / clip / panel (8 transport group) |
| border | 1 | focus ring 2 + 2 offset, accent |
| shadow.offset | 2 2 0 | rgba(0,0,0,.35) light: rgba(20,24,28,.22) |
| motion.hover / press | 80 / 40 ms | ease-out |
| motion.panel | 160 ms | ease-in-out, max 200 |
| motion.meter | att 0 / dec 20 dB/s | peak hold 1.5 s |
| reduced motion | 0 ms | no panel or hover transitions; meters still decay |

### 3.5 Clip palette (12 hues, from the design's OKLCH recipe)

| Hue | Dark: body / header / waveform / border (waveform contrast) | Light: body / header / waveform / border (waveform contrast) |
|---|---|---|
| Coral (25°) | `#5a2a28` `#873f3b` `#ffb1aa` `#a45953` (6.75:1) | `#ffc6c0` `#f8a49d` `#682321` `#bb6d67` (7.61:1) |
| Amber (55°) | `#562f12` `#824717` `#feb98b` `#9e6033` (6.92:1) | `#fdccac` `#f0ad7f` `#632b00` `#b47548` (7.68:1) |
| Olive (85°) | `#4b3702` `#715200` `#e9c67d` `#8c6c1f` (6.95:1) | `#edd5a3` `#dbb970` `#553700` `#a28137` (7.58:1) |
| Lime (115°) | `#3a3e0c` `#575e0a` `#c8d386` `#70782c` (7.0:1) | `#d5dea9` `#bbc679` `#3c4200` `#848d42` (7.53:1) |
| Green (150°) | `#1b4526` `#266739` `#9bdda8` `#428252` (6.91:1) | `#b7e5bf` `#8ed09c` `#004b1e` `#579766` (7.39:1) |
| Teal (180°) | `#00463c` `#00695a` `#7ae0cd` `#008474` (6.89:1) | `#a4e7d9` `#6dd3c0` `#004d3f` `#2c9a88` (7.04:1) |
| Cyan (205°) | `#00444c` `#006772` `#73dee9` `#00828d` (6.91:1) | `#a0e5ed` `#65d0dc` `#004a56` `#1d97a3` (7.06:1) |
| Blue (235°) | `#07405a` `#006087` `#87d5ff` `#287aa3` (6.87:1) | `#abe0ff` `#7ac8f5` `#004369` `#408fba` (7.39:1) |
| Indigo (265°) | `#28395f` `#3c5590` `#abcaff` `#5570ac` (6.86:1) | `#c1d8ff` `#9ebdff` `#233971` `#6885c3` (7.67:1) |
| Violet (295°) | `#3d325b` `#5d4b8a` `#cfbeff` `#7665a6` (6.9:1) | `#dacfff` `#c2b1f8` `#42306b` `#8a7abc` (7.69:1) |
| Magenta (325°) | `#4d2d4e` `#744377` `#ecb4ef` `#8f5c92` (6.85:1) | `#f0c8f1` `#dfa8e1` `#562759` `#a571a7` (7.78:1) |
| Rose (355°) | `#562a3c` `#823e5b` `#ffafce` `#9f5774` (6.85:1) | `#fec5da` `#f2a3c1` `#632240` `#b56c89` (7.75:1) |

Role names (body, header, waveform, border) are inferred from the design's variable names.

## 4. Redlines (logical px)

| Part | Values |
|---|---|
| **Track header** | 232 × 74 (compact 40 high); a 4 px strip at the left (inferred to be the track-colour bar); 8 px padding; the 74 px stack top to bottom is 6 + 18 + 4 + 20 + 4 + 16 + 6; track buttons 22 × 18 in the header (28 × 22 in the inspector) |
| **Clip** | height 64 (example width 330); header 14; fade zone 14; fade handle 6 |
| **Transport** | buttons 40 × 40; play 52 × 40; counter 316 × 48; 20 gap; counter numerals 24 / 28 (bars) and 16 / 20 (time) |
| **Fader** | states default, at a value, dragging (cap outlined, value tooltip), disabled (track inactive), automated (cap outlined, fill dashed, blue). The board labels "0 dB at 91 px", **but its own value formula gives about −22.5 dB at that position, so the mockup's scale is a placeholder**: use the app's real fader law, not the drawn positions |
| **Pan** | centre, left, right, dragging (2 px accent ring), disabled, automated |
| **Automation modes** | Off –, Read ▶, Write ●, Touch ◐, Latch ◉ (each has a glyph and a label, and Latch has a dashed border) |

## 5. Screens and the plan tasks they feed

| Board | Plan task |
|---|---|
| 3b Assistant panel | P1-I01 |
| 3a Vocal inspector, 4a Vocalist workspace | P1-I02 |
| 3c Change review | P1-I03 |
| 3d Audition | P1-I04 |
| 3e Task history | P1-I05 |
| Keyboard, contrast, no colour-only meaning (all boards) | P1-I06 |
| Design system theme and fonts; Arrange window restyle (Arrange Pro, 4b, 4c, 5a to 5c) | P1-I07, P1-I08 (proposed) |
| 3f Recording health (live status and disk-full dialog) | P3-R03 |
| 3g Cloud opt-in, first run, missing media | P1-M03, P1-M04 (opt-in, budget), P3-P01 |

## 6. Interaction notes

**The Change review row below is overridden by decision UI-1 in [decisions.md](decisions.md)** (a production proposal has Preview and Reject only; Approve is for explicit assignments). The rest is as designed.

| Component | Click, hover, keyboard | Disabled or unavailable |
|---|---|---|
| Button | Hover raises the gradient 8%. Press inverts it for 40 ms. Enter or Space activates. Focus shows a 2 px accent ring. | Dimmed to 70%. Hover shows a tooltip that says why, for example “Select a range first”. |
| Toggle | Click flips on and off. The on state adds a check and a fill, so it is not colour only. Mixed shows a dash. | Same as button. |
| Transport | Space plays and stops. Return goes to start. Clicking record while armed starts recording and the button turns solid red with the word “Recording”. | Record is off with “Choose an input first” when no track is armed. |
| Track buttons | R arms, I monitors, M mutes, S solos. Option-click on S sets solo-safe (outline). Another solo shows other tracks as dashed “implicitly muted”. | On an inactive track all four are off and say “Track is inactive”. |
| Fader | Drag to change. Shift-drag is fine adjust. Double-click resets to 0 dB. While dragging a tooltip shows the value. Arrow keys move 0.1 dB, Shift 1 dB. | When automation is in Read the cap is outlined and moves by itself. Dragging then asks to switch to Touch. |
| Pan | Drag up or down. Double-click centres. Arrow keys move one step. | Same as fader. |
| Meter | Click the clip box to clear the latched clip. Hover shows peak and RMS in dB. | A track that is off shows an empty well with no fill. |
| Clip | Click selects with a 2 px outline. Drag moves. Drag an edge to trim. Hover near the top corners shows fade handles. Double-click opens the clip in the editor. | Locked clips show a dashed outline and refuse drag with the message “Clip is locked”. Missing media shows “Relink…”. |
| Counter | Click to edit. Enter confirms. Escape cancels. Tab switches between Bars|Beats and Min:Secs. | Read-only while recording. |
| Ruler | Click sets the playhead. Drag selects a range. The Ruler button in the corner collapses it to one row. | None. |
| Track header | Click selects and shows it in the inspector. Double-click the name to rename. Drag the bottom edge to change the height. | Inactive: dimmed, labelled “Inactive”, controls off. |
| Insert slot | Click the power dot to bypass. Click the name to open. Drag to reorder. Empty slots open a plug-in list. | Bypassed shows a struck-through name and the word “Bypassed”. |
| Vocal inspector | Drag on the waveform to select a range. Handles move one edge. The readout shows start, end and length. Choose a target note and “Use current selection” to set a practice region. | With no range it shows “Select a range on the lead vocal to see its pitch.” Practice results appear only for a chosen region. |
| Nearest-degree tag | Hover shows “Nearest note to the measured pitch. This is not a grade.” Tags are not clickable. | Hidden where confidence is low. |
| Assistant | Enter sends. Shift-Enter adds a line. Evidence links move the playhead and highlight the range they refer to. Cancel stops work and changes nothing. | When the cloud assistant is off the request box is off and says why. Goal chips still work. |
| Change review | Editing any value turns the preview stale and enables Render. Approve is off while the preview is out of date. Reject needs no confirmation. | Approve is off with the message “Render the preview first” when the preview is stale. |
| Audition | Space switches A and B. The order is random for each audition. Loudness matching affects listening only. Keep this change, Keep the original and Discard have equal size and weight. | When the session changes, Keep this change turns off and the strip says why. The other two stay on. |
| Task history | Click a row to open its receipt. Undo is available only for Applied tasks. Recovery required offers three actions with the evidence beside them. | Undo reads “Not needed” or “Not available” instead of a blank. |
| Recording health | The status bar is always visible while recording. Details opens the take list. The disk-full dialog takes focus and needs a click to close. | Not applicable. |

## 7. Microcopy

The exact wording is in [copy.json](copy.json), in seven groups:

- **5.4 Assistant panel**: 10 strings
- **5.5 Change review**: 10 strings
- **5.6 Audition**: 7 strings
- **5.7 Task history**: 12 strings
- **5.8 Recording health**: 7 strings
- **5.9 Vocal Check and cloud**: 12 strings
- **Missing media and empty states**: 8 strings

Numbers in the strings (levels, times, loudness differences, the rule version) are **samples**: fill them from real values at build time.

## 8. Gaps against the brief

- **No icon set** (brief §6: about 40 icons at 20 and 16 px as SVG). The boards use a few inline glyphs and Unicode symbols only.
- **Component state sheets are incomplete.** The design system sheet covers button, toggle, transport, track buttons, text field, tabs, insert slot and counter. Dropdown, slider, context menu, tooltip, dialog, toast, banner, progress, collapsible section and meter detail appear only inside screens, if at all.
- The "Paper" direction was dropped, which was the intent of round 1, so nothing is missing there.

## 9. Issues to settle before building

1. **Two approvals for one change: decided.** The design had Preview, Approve and Reject in the change review and then "Keep this change" in the audition. PRD v0.3 section 5.2 makes **Keep in the audition the approval for production proposals** and allows approval from the change list only for simple supported edits. Decision UI-1 in [decisions.md](decisions.md): production proposals have **Preview and Reject only**, explicit assignments have **Approve and Reject**, and the controller sets the path. Where this differs from the bundle's strings, decisions.md wins.
2. **Placeholder rule version.** The copy says "version 1.2"; the headroom rule is `level.headroom` 0.1.0. The string must be filled from the rule at build time.
3. **The microcopy has no wording for the policy's other results** (stale analysis, role not confirmed, automation conflict, too little signal, near-silence, fader limits). The policy returns a code for each ([production-policy.md](../production-policy.md)); each needs a plain-language message in the same style. Ask the design tool for them or write them to the same rules.
4. **Dense hit targets.** The dense sizes (22 × 18 track buttons) are smaller than egui's default hit targets; they need explicit sizing and a check that they remain operable.

## 10. Implementation notes for egui

Straightforward: filled and rounded rectangles, 1 px borders, vertical gradients, a 1 px inset highlight line, the offset shadow (2, 2, 0 at a translucent black, as the design states), custom-painted meters, waveforms and pitch contours, the 2 px focus ring with a 2 px offset, and a fixed-width mono face for numbers. Needs care: bundling and registering the two font families, a live dark/light switch, the exact dense sizes, panel minimum and maximum widths (inspector 200 to 320, right list 160 to 320, bottom panel 32 to 400), the 40 ms press inversion and 80 ms hover timings, the reduced-motion rule (no panel or hover transitions; meters still decay), and Retina scaling.

## 11. Provenance and licence

The pages were generated by Claude Design from the brief. They contain no `<img>` tags and no embedded image data: everything is inline SVG and CSS. The only external references are Google Fonts stylesheets (used when the pages are viewed, not by the app). The fonts are named OFL families (§3.3). The bundled runtime script was not used and is not redistributed.

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Digest of the received bundle, with extracted tokens and copy. |
