# UI design specification (digest of the Claude Design bundle)

**Version:** 0.3 · **Date:** 15 October 2026 · **Status:** Rounds 1 to 6 received, the eight requested fixes checked. **Implemented so far (16 October 2026):** the Graphite palette, IBM Plex fonts, OKLCH clip colours and icon set, and in the track header the plain accent-coloured name and the pan knob, horizontal fader and level readout (on top of upstream's header, which keeps its extra controls). **Not implemented:** the vocal inspector (3a), assistant panel (3b) and change review (3c), and the rest of the Arrange window redlines.
**Source:** the Claude Design bundle "Arrange Window Design Round 1" (rounds 1 to 6; the second download, with `v2` pages that supersede the originals), made from [ui-design-brief.md](../ui-design-brief.md) and [round-6-brief.md](round-6-brief.md).
**Decisions that override the bundle:** [decisions.md](decisions.md).
**Machine-readable:** [tokens.json](tokens.json) (colours, type, sizes, palette, redlines, hit areas, fader law), [copy.json](copy.json) (interaction notes and exact wording), [icons.json](icons.json) (63 icons as path data) and [messages.json](messages.json) (the 15 messages from board 6b). Both are generated from the bundle as data by `tools/design/extract_design.py`; nothing in the bundle is executed.

## 1. What this is, and what is not in the repository

The bundle is a set of design-canvas pages (`*.dc.html`) plus a third-party runtime script (`support.js`) that draws them. **Neither the zips, `support.js`, the 126 icon SVG files nor the design tool's screenshots are committed**: the runtime has no licence header, the pages are an authoring format, every SVG carries an embedded provenance block (about 8 KB each, not needed), and images in the repo need attribution rows. `icons.json` carries the same path data. This document and the two JSON files carry what an implementer needs. **To import a new version:** unzip the bundle into a new, empty folder; run `python3 -I tools/design/verify_bundle.py BUNDLE_DIR` (it checks the icons, the SVGs, the fader law against the app's, the message style, and whether the fixes asked for in §9 actually landed; it exits non-zero on any FAIL and lists what still needs a human look); then run `python3 -I tools/design/extract_design.py BUNDLE_DIR docs/soundcraft-ai/design` to regenerate the four JSON files. Neither script runs anything from the bundle.

**Direction chosen:** the dark **"Graphite"** direction (dense, teal accent), with **IBM Plex Sans** for the interface and **IBM Plex Mono** for numbers, plus a full light theme. The alternative from round 1 ("Paper": warm light, vermilion accent, Figtree and JetBrains Mono) was not carried forward.

**What was and was not checked.** I read the bundle's data and markup as text and recomputed every contrast ratio and the clip palette (§3); all 112 interaction and copy strings and all 30 message strings were checked verbatim against the source; the 63 icons were rendered and looked at (§4.3); the fader law was compared with the app's real law (§4.2). I looked at two of the design tool's own renders (3c v2 and 6e) but **did not render the other boards**. Fidelity will be checked by rendering the built app with `ui_shot` next to the boards.

## 2. Boards

| Board | Content | Size |
|---|---|---|
| Round 1: 1a, 1b | Two directions for the Arrange window | 1440 × 900 |
| Arrange Pro v2, Arrange Pro Light v2 | The chosen direction, dark and light | 1440 × 900 |
| Design System v2 | Colour, type, sizing, motion, clip palette, component states | n/a |
| 3a Vocal inspector | Selected-note practice active | 1440 × 900 |
| 3b Assistant panel | Suggest, running, unavailable | 360 wide (min 320, max 480) |
| 3c Change review (v2) | Three result kinds; a production proposal shows Preview · Reject only | 440 wide (min 400) |
| 3d Audition | A/B, matching, keep, keep original, discard; keyboard focus starts on the A/B switch (ring drawn in the third download) | 1040 × 640 (min 800 × 520) |
| 3e Task history (v2) | All task states, with rejection reasons; "Ready for approval" = audition open | 1040 wide (panel min 640) |
| 3f Recording health | Status bar 40 px, always visible; warning, error, recovery | n/a |
| 3g First run, cloud opt-in, missing media | | n/a |
| 4a Vocalist workspace | One record button, large waveform, pitch view | 1440 × 900 |
| 4b Redlines | Track header and clip at 3×, transport at 2× | logical px |
| 4c States | Fader, pan, automation, track types | n/a |
| 5a Arrange, light | | 1440 × 900 |
| 5b Adapting | 1280 × 800 and 2560 × 1440 (scaled schematics) | n/a |
| 5c Track types | Folder (open), instrument, hidden, aux; header 232 × 74, compact 232 × 40 | n/a |
| **6a** Change review | Production proposal (Preview · Reject) and explicit assignment (Approve · Reject), dark and light | 440 wide |
| **6b** Messages | 15 messages, each with a key, text, what the user can do and where it is shown | n/a |
| **6c** Icon set | 63 icons, drawn natively at 16 px and 20 px, 1× and 2×, both themes | n/a |
| **6d, 6dm** Component state sheets | Dropdown, slider, context menu, tooltip, dialog, banner, progress, meter, scroll bar, field and more; **6dm is a state matrix** (default, hover, pressed, focus, disabled, n/a with reason) | n/a |
| **6e** Fader scale and hit areas | The real fader law at three heights; drawn versus clickable sizes | n/a |
| **6f** Design System additions | New components and tokens (the design lists new components only; no new colour tokens) | n/a |
| **6x** Conflicts | What in rounds 1 to 5 conflicts with round 6 | n/a |
| Notes and Copy v2 | Interaction notes and microcopy (UI-1 applied) | n/a |

The original (non-v2) pages were kept in the bundle and flagged as conflicting where they are; use the v2 pages and board 6a.
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
| **Fader** | see §4.2 for the scale. States default, at a value, dragging (cap outlined, value tooltip), disabled (track inactive), automated (cap outlined, fill dashed, blue). Round 4's scale was a placeholder (its "0 dB at 91 px" label disagreed with its own formula); **6e replaces it with the real law** |
| **Pan** | centre, left, right, dragging (2 px accent ring), disabled, automated |
| **Automation modes** | Off –, Read ▶, Write ●, Touch ◐, Latch ◉ (each has a glyph and a label, and Latch has a dashed border) |

### 4.1 Hit areas (board 6e; drawn size versus clickable size, logical px)

| Control | Drawn | Hit area | Note |
|---|---|---|---|
| Track buttons, header | 22 × 18 | 26 × 24 | Gap 4 between buttons. |
| Track buttons, inspector | 28 × 22 | 32 × 28 | Gap 4. |
| Insert power dot | 12 × 12 | 24 × 24 | Within a 24 px slot row. |
| Insert slot | full × 24 | full × 24 | Rows touch; no extra padding. |
| Fade handle | 6 × 6 | 16 × 16 | Appears on hover near the clip corner. |
| Meter clip box | 14 × 8 | 24 × 16 | Click clears the latch. |
| Pan knob, header | 20 × 20 | 24 × 24 | Drag vertically. |
| Fader cap, header | 10 × 14 | 18 × 24 | The whole groove row (20 px high) also accepts a click to jump. |
| Fader cap, inspector | 24 × 36 | 24 × 36 | Already large enough. |
| Disclosure arrow | 10 × 10 | 24 × 24 | The whole panel header row is also clickable. |

Minimum gap between neighbours 4 px; the focus ring is 2 px at a 1 px offset so it stays inside the gap.

### 4.2 The fader law (checked)

The design's 13 points match the app's real law (`soundcraft_model::fader_db_to_pos`) to within 0.0005, so the scale in 6e can be trusted: 0 dB at 75% of the travel, +6 at 87.5%, +12 at the top, −24 at 25%, then compressing toward −∞. **The app must use the model function, not a copy of these points.** Labels by height: 120 px travel shows +12, 0, −12, −24, −∞; 190 px shows +12, +6, 0, −6, −12, −24, −48, −∞; 260 px shows ten marks (the drawn list omits −48, although its caption says "all marks from +12 to −60"; the tightest gap is 14 px). Unity (0 dB) is the one mark with an accent tick and a bold label.

### 4.3 Icons (board 6c; data in [icons.json](icons.json))

**63 icons** in 7 groups (transport 8, tool 7, mode 4, view 5, track 17, product 11, status 11), every item the brief asked for, each drawn at **20 px** (grid 20, padding 2) and **16 px** (grid 16, padding 2). **Stroke: 1 px at both sizes, round caps and joins**, and the group is offset by half a pixel so a 1 px line is crisp at 1× and 2×; the design chose this because egui draws 1 px strokes. The 16 px set is genuinely redrawn, not scaled (61 of 63 differ from the 20 px geometry by more than scaling, the other two being a square and an X; 98% of 16 px coordinates are on whole pixels; the design notes where 16 px drops detail, such as the grid going from 4 × 4 to 2 × 2). **A path renderer needs only `M L H V A C S Z`** (absolute and relative; arcs are circular). Four icons also have a **filled** path (record, record-arm, automation-write, overflow), which the renderer must draw as filled circles.

The design's own originality notes: the automation glyphs (a wave with a triangle, dot, ring or barred ring) are "not a standard"; the selector (a cornered marquee) and the magnifier family are near-universal conventions; it lists every generic shape it used (transport, padlock, eye with a slash, curved undo and redo, warning triangle, information, error, success, speech bubble, sliders, power symbol, headphones and so on). Whether that is acceptable is the project owner's call.

## 5. Screens and the plan tasks they feed

| Board | Plan task |
|---|---|
| 3b Assistant panel | P1-I01 |
| 3a Vocal inspector, 4a Vocalist workspace | P1-I02 |
| 3c Change review | P1-I03 |
| 3d Audition | P1-I04 |
| 3e Task history | P1-I05 |
| Keyboard, contrast, no colour-only meaning (all boards) | P1-I06 |
| Design system theme and fonts; Arrange window restyle (Arrange Pro v2, 4b, 4c, 5a to 5c) | P1-I07, P1-I08 (proposed) |
| Icon set in code (6c, `icons.json`) | P1-I09 (proposed) |
| 3f Recording health (live status and disk-full dialog) | P3-R03 |
| 3g Cloud opt-in, first run, missing media | P1-M03, P1-M04 (opt-in, budget), P3-P01 |

## 6. Interaction notes

From Notes and Copy v2, which already applies decision UI-1 (the change review has separate rows for a production proposal and an explicit assignment).

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
| Change review · production proposal | Preview is the primary action and Reject sits beside it. There is no Approve. Preview renders the candidate and opens the audition, where Keep this change is the single approval. Editing any value turns the preview stale. | Preview is off with the reason “A plug-in in the signal path cannot be rendered here.” when the plan cannot be rendered. |
| Change review · explicit assignment | Approve is primary and Reject sits beside it. There is no preview and no audition. Editing any value turns approval off. | Approve is off when the session changed: “The session changed. Ask again to review this change.” |
| Audition | Focus starts on the A/B switch. Space switches A and B. The order is random for each audition. Loudness matching affects listening only. Keep this change, Keep the original and Discard have equal size and weight. | When the session changes, Keep this change turns off and the strip says why. The other two stay on. |
| Task history | Click a row to open its receipt. Undo is available only for Applied tasks. Recovery required offers three actions with the evidence beside them. | Undo reads “Not needed” or “Not available” instead of a blank. |
| Recording health | The status bar is always visible while recording. Details opens the take list. The disk-full dialog takes focus and needs a click to close. | Not applicable. |

## 7. Microcopy

The exact wording is in [copy.json](copy.json) (seven groups, 72 strings) and, for the policy results, in [messages.json](messages.json) (15 messages):

- **5.4 Assistant panel**: 10 strings
- **5.5 Change review**: 13 strings
- **5.6 Audition**: 7 strings
- **5.7 Task history**: 15 strings
- **5.8 Recording health**: 7 strings
- **5.9 Vocal Check and cloud**: 12 strings
- **Missing media and empty states**: 8 strings

Numbers in the strings (levels, times, loudness differences, the rule version) are **samples**: fill them from real values at build time.

## 8. What the round 6 bundle resolved

- **Icon set:** 63 icons at both sizes, as specified (§4.3).
- **Component states:** the 6d sheets name dropdown, slider, context menu, tooltip, dialog (including the disk-full dialog), banner and toast, progress, collapsible section, panel header, meter, scroll bar and text field, and 6dm is a state matrix (default, hover, pressed, focus, disabled, with "n/a" and the reason where a state does not apply). **The matrix's completeness is the board's claim; I did not check it cell by cell.**
- **Approval flow:** the v2 pages apply decision UI-1 (3c, 3e, the notes and the copy).
- **Fader scale:** redrawn to the real law (§4.2). **Hit areas:** drawn versus clickable sizes for ten dense controls (§4.1).
- **Messages:** all 11 situations from the round 6 brief plus the three UI-1 strings, 15 messages in all, each saying whether anything changed.

## 9. Issues found in review, and what is left

**Fixed in the third download** (checked with `tools/design/verify_bundle.py` and by rendering):
1. **`zoom` was a near-duplicate of `zoom-out`.** It is now a lens with a solid dot, clearly different from zoom-in, zoom-out and zoom-selection at 6× and at true size.
2. **`smart-tool` was a four-pointed sparkle** (reads as "AI"). It is now a pointer with a small ring: no sparkle, distinct from the marquee selector.
3. **`stale` read as a sigma.** It is now a clock, clearly different from `recovery` (a circular arrow with a bar). A clock says "time" rather than "out of date", but it is unambiguous and legible.
4. **`preview_unavailable_plugin`** now says bypassing the plug-in would make the proposal out of date, and tells the user to bypass it and ask again.
5. **`goal_unsupported`** now takes a `{supported_goals}` placeholder, to be filled from the goals actually available at build time.
6. **Board 3d** now draws the keyboard focus ring (2 px accent, 4 px offset) on the A/B switch, with the note "Keyboard focus starts on this switch".

**Withdrawn.** I had listed a mismatch between the 6e caption and the labels drawn on the 260 px fader. That came from the design tool's older screenshot; the page's own caption already said that −48 is left unlabelled (so labels stay 14 px apart) and it matches the drawn labels. Not an issue.

**Still open, all minor:**
7. **`proposal_failed` has three sentences** (the style rule was two). It is a reassurance line ("It was not caused by anything you did"); keep or trim.
8. **1 px strokes at 20 px, and latch against touch at 16 px,** look legible in the design tool's true-size tiles and in my true-size renders on both themes, but must be checked in the built app at 1× before they are accepted.
9. **The design's own conflict list (6x)** is kept in `tokens.json` under `design_listed_conflicts_with_older_boards`. The v2 pages fixed 3c, 3e and the notes and copy; the Round 4 and Round 2 header fader, the sizes table and the icon list should be read through 6e, §4.1 and 6c, not their older pages.
10. **The imitation call** on the icon set is the project owner's: the design lists the generic shapes it used and describes the automation glyphs as its own. The new smart-tool pointer is itself a common cursor shape.

**Build and licence notes:**
11. The SVG exports embed a **C2PA content-credentials block** (about 8 KB each). `icons.json` drops it.
12. **Dots and discs** (overflow, record, record-arm, automation-write) and the new zoom dot are filled paths, not strokes; the renderer must fill them.
13. **Dense hit targets** are specified (§4.1); the app's controls must implement the hit area, not just the drawn size.

## 10. Implementation notes for egui

Straightforward: filled and rounded rectangles, 1 px borders, vertical gradients, a 1 px inset highlight line, the offset shadow (2, 2, 0 at a translucent black, as the design states), custom-painted meters, waveforms and pitch contours, the 2 px focus ring with a 2 px offset, and a fixed-width mono face for numbers. Needs care: bundling and registering the two font families, a live dark/light switch, the exact dense sizes, panel minimum and maximum widths (inspector 200 to 320, right list 160 to 320, bottom panel 32 to 400), the 40 ms press inversion and 80 ms hover timings, the reduced-motion rule (no panel or hover transitions; meters still decay), and Retina scaling.

## 11. Provenance and licence

The pages were generated by Claude Design from the briefs. They contain no `<img>` tags and no embedded image data: everything is inline SVG and CSS. The only external references are Google Fonts stylesheets (used when the pages are viewed, not by the app). The fonts are named OFL families (§3.3). The bundled runtime script was not used and is not redistributed.

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Digest of the received bundle, with extracted tokens and copy. |
| 0.2 | 15 October 2026 | Round 6 imported (v2 pages, icons, messages, state sheets, fader law, hit areas); icons and fader checked; open issues rewritten. |
| 0.3 | 15 October 2026 | Third download imported: zoom, smart tool and stale icons replaced, two messages fixed, focus ring drawn on 3d; one issue withdrawn; remaining items are minor. |
