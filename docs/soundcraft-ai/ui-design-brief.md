# SoundCraft UI design brief (for Claude Design)

**Version:** 0.1 · **Date:** 14 October 2026 · **Status:** Draft for you to review before it goes to a design tool.
**Attach with it:** `soundcraft-current-ui.png` (the app today, rendered from the demo session).

## 0. How to use this brief

Work in rounds. Do not design everything at once.

1. **Round 1: one slice, two directions.** Design the **Arrange window** (§5.1) in **two clearly different visual directions** at 1440 × 900. Include the track header, the clip, the transport and toolbar. We pick one.
2. **Round 2: the design system.** For the chosen direction: tokens (§4), the component sheet (§5.2) with every state, and the icon set (§6).
3. **Round 3: the new product screens** (§5.3 to §5.9): the assistant, the vocal inspector, change review, audition, task history, recording health and the privacy screen.

Give every deliverable in the format in §7, so a developer can implement it exactly.

## 1. The product

**SoundCraft** is a desktop audio workstation (a DAW) for macOS on Apple Silicon: record, edit and mix audio. Functionally it follows the Pro Tools model (Edit and Mix windows, tracks, clips, a transport, a grid). It is open source and written in Rust.

**SoundCraft-AI** is the new layer. A **home vocalist** records or imports a lead vocal over a backing track, then:
- inspects the pitch of their singing,
- gets a **proposal** (for example "lower the vocal level by 3.7 dB to leave headroom"),
- **auditions** it level-matched against the original,
- and **keeps** it or **keeps the original**, with a reliable undo.

The AI is an assistant, not an autopilot. The user decides. The design must make that feel true.

## 2. Users

- **Primary:** an independent vocalist recording at home, with a backing track. They know their music and hear well, but they are not a recording engineer. They need pitch feedback and help with level, tone and compression, without jargon.
- **Secondary:** people who already use a DAW. They should find tracks, clips, a transport and a mixer where they expect them.

## 3. Design principles

1. **Calm and precise.** A tool people sit in for hours. Restraint beats decoration: few colours, clear hierarchy, nothing that moves without a reason.
2. **Honest.** Show what was measured, what is inferred, and what is a suggestion. Show uncertainty. Never present a score as a probability or a suggestion as a fact.
3. **The user is in control.** "Keep the original" and "Discard" have equal visual weight to "Keep". Nothing changes the session without an explicit action. Everything can be undone.
4. **Dense, but legible.** Pro-audio density with a strict spacing grid. Never rely on colour alone.
5. **Original.** Do not imitate or reproduce another product's look, icons or artwork (see §8).

## 4. Foundations to specify

Name tokens in `snake_case` with a dot hierarchy so they map to code, for example `bg.surface.1`, `text.primary`, `accent.record`.

| Area | What I need |
|---|---|
| **Colour** | A dark theme (default) and a light theme. Background layers (at least 4 elevations), text (primary, secondary, disabled), borders, accent, and status colours (info, success, warning, error). **Transport/record colours** (play, record armed, solo, mute). **A track colour palette** of 12 hues that work as clip fills in both themes. Give hex values and **measured contrast ratios**. |
| **Typography** | Up to two open-licensed families (SIL OFL or Apache-2.0; state the licence and where to download). A numeric style with **tabular figures** for the counter and meters. A type scale (size, weight, line height) for: window title, panel heading, label, body, caption, mono/number. |
| **Spacing and sizing** | A base unit (4 px) and a scale. Heights for: toolbar, track header (normal and compact), ruler row, panel header, buttons (small and normal), text fields. Minimum click target. |
| **Shape and depth** | Corner radii (control, clip, panel). Borders. Shadows are limited to **a single offset translucent layer**: no blur (see §9). |
| **Motion** | A short list: durations and easing for hover, press, panel open, meter decay. Maximum 200 ms for UI transitions. A reduced-motion behaviour. |
| **Focus** | A visible keyboard focus ring for every interactive element. |

Accessibility: text contrast at least 4.5 : 1, controls and meaningful graphics at least 3 : 1, in both themes. **State is never carried by colour alone**: add a shape, label or icon. Full keyboard operation.

## 5. Screens and components

Design at **1440 × 900** (and show how it adapts down to 1280 × 800 and up to 2560 × 1440). Retina 2x assets are assumed.

### 5.1 Arrange window (restyle of the existing screen) — Round 1

Today (see the attached screenshot): a menu bar; a toolbar with edit-mode buttons (Shuffle, Spot, Slip, Grid), zoom buttons, tools, a large counter, grid and nudge settings, and transport; a left tracks-and-groups list; stacked ruler rows (Bars|Beats, Min:Secs, Tempo, Meter, Markers); track rows with a header and coloured clips; a right clip list; bottom tabs (MIDI Editor, Memory Locations, Undo History); and a status line.

What is weak today, so the redesign should fix it:
- Track headers are rows of flat, similar-looking widgets. The fader and pan are shown as text (`vol -3.0 pan 0`), inserts as plain text, meters are barely visible.
- Clips are flat saturated rectangles with heavy black waveforms and no visible fade handles.
- Toolbar buttons are small and low contrast; the coloured toggle blocks look like debug chips; there is no grouping or clear hierarchy.
- Five stacked ruler rows consume vertical space.
- There is no inspector or channel strip for the selected track.

Design: the toolbar and transport (with a large, clear counter), a **ruler** that can collapse to fewer rows, the **track header**, the **clip** (normal, selected, muted, and with fades), the **left list**, the **right clip list**, the **bottom tab bar** and the **status line**. Optionally an **inspector pane** showing the selected track's channel strip (name, input, fader, pan, meter, inserts, sends).

### 5.2 Components, every state

For each: **default, hover, pressed, focus, disabled**, plus the extra states listed.

| Component | Extra states and notes |
|---|---|
| **Button** (icon, text, icon + text), **toggle** | on, off, mixed. Sizes small and normal |
| **Transport buttons** | play (active), stop, record (armed, recording), loop, rewind and forward, return to start |
| **Track buttons** | record arm (armed, recording), input monitor, **solo** (soloed, solo-safe, implicitly muted by another solo), **mute** (muted, implicitly muted) |
| **Track header** | normal and compact heights; selected; **inactive**; **hidden**; folder track; aux/bus; instrument; with a live **meter**; with a **fader**; **an insert list** showing bypassed and empty slots; **automation mode** (read, write, touch, latch) |
| **Fader** and **pan knob/slider** | at default, at a value, dragging (with a value tooltip), disabled, automated (follows the automation) |
| **Level meter** | mono and stereo, **peak and RMS**, **clip indicator** (latched), peak-hold, scale, dark and light themes |
| **Clip** | normal, selected, muted, locked, **reduced waveform detail at low zoom**, fade handles, **missing media**, MIDI clip with notes |
| **Counter** | Bars|Beats and Min:Secs, editable |
| **Text field, dropdown, spinner, slider** | with units; invalid; read-only |
| **Tabs, panel header, collapsible section** | |
| **Context menu, tooltip, dialog, toast, banner** | banner levels: info, warning, error |
| **Progress** | determinate, indeterminate, **cancelling** |

### 5.3 Vocal inspector (new) — Round 3

The most important new view. Shows the singer's **pitch** over time for the selected range of the lead vocal.

Elements:
- The **waveform** of the range.
- The **measured pitch contour**: a continuous line in Hz and cents, **relative to Sa** (the singer's chosen reference note), with the vertical axis labelled in note or swara names. **Gaps where the voice is unvoiced** (no line). Slides and vibrato must look like what they are: never snapped or stepped.
- **Nearest-degree labels** (the scale degree closest to the pitch): clearly secondary, small, near the line. They are **proximity, not correctness**. They must never look like a grade or like the singer was "wrong".
- An optional **scale overlay** as faint horizontal guides.
- **Uncertainty** as a translucent band or a lighter line segment where the tracker is less sure, and a clear way to see **where the data is unreliable** (breathy or noisy passages) without implying poor singing.
- **Selected-note practice**: the user picks a **target swara, octave and a sustained region**. Show the target as a distinct horizontal marker and region, and report **cents deviation, stability and confidence for that region only**, with transitions visibly excluded.
- **Sa and octave control**; a precise **range selection** with a time readout; zoom.
- A header showing the **tuning** (named, for example "equal temperament") and an **Advanced** area (hidden by default) for custom cents offsets and timed MIDI comparison.

The design must make three things visibly different, because users must be able to explain the difference: **what was measured** (the line), **what is nearest** (the labels), and **the error from a target they chose** (the target marker and the deviation readout). Use shape, position and label as well as colour.

**Confidence is shown as a small number of levels (for example low / medium / high) with a legend, never as a percentage**: the scores are not probabilities.

### 5.4 Assistant panel (new)

A side panel. Elements: the **scope** (which track and range, as removable chips, with the target's name and ID), the **mode** (**Explain**: reads only; **Suggest**: builds a proposal to review; **Apply**: executes an approved plan; the mode never changes by itself), a **request box** with example goals ("headroom", "vocal clarity", "even level"), **progress with cancel**, and **responses with evidence links** that point at the measurement or the contour region they refer to. A clear **unavailable** state with the reason ("cloud assistant is off; local goal controls still work"). **Users never see raw JSON**; technical details sit behind an "details" disclosure.

### 5.5 Change review (new)

Shown before anything is applied. A proposal lists, per change: the **target** (track name and ID), the **parameter**, **before and after values with units**, **the rule that produced it and its version**, the **measurements used (with units and where in the signal they were taken)**, and the **expected effect** ("true peak moves from −2.3 to −6.0 dBTP"). Three result kinds must look different:
- **Proposal** (there is a change to review),
- **No change needed** ("already within 0.5 dB of the target"),
- **Can't propose** ("the source is clipped, so raising the level would not repair it"), with a **plain-language reason** and what the user can do.

Also show **limits applied** (for example "limited to 12 dB per change"), **notes** (for example "source is clipped"), and **edit controls** that keep the change within allowed bounds. Editing anything **invalidates a previous preview**; make that state visible ("Preview out of date: render again").

### 5.6 Audition view (new)

Compare the original with the candidate. Elements: **A/B switch** (with the order randomised and not labelled "better"), **solo** and **in-context** listening (with the backing track), a **loudness-match indicator that is always visible** and says whether the two versions are matched (and by how much), a warning when the candidate **introduces new clipping**, a **stale** state if the session changed, and three equal-weight actions: **Keep**, **Keep the original**, **Discard**. Keep is **never** the visual default or the loudest button. The matching applies only to listening; show that it is **not applied to exports**.

### 5.7 Task history (new)

A list of tasks: the request, the **state**, what changed, the outcome, and whether **undo** is available. States to distinguish with shape and label (not only colour): Draft, Validated, Previewing, Ready for approval, Approved, Committing, Applied; and Rejected, Cancelled, **Stale**, **Failed**, and **Recovery required** (the outcome is unknown; show the evidence and what the user can do). A task that was only **proposed** must never look like one that **happened**. A detail view with the full receipt, collapsed by default.

### 5.8 Recording health (new, important)

During recording, a **live status** that is always visible: recording (with elapsed time), the input in use, and a clear **warning state** if input is being lost ("0.8 s of input lost: the disk is too slow") and an **error state** if the take can no longer be written ("Recording stopped: the disk is full. Your take so far is kept."). The error must interrupt, be impossible to miss, and say what is kept. Also the **after-a-crash** recovery message ("A take was recovered: 2:41 of audio").

### 5.9 First run, empty and privacy screens

- **Vocal Check flow:** import or record a lead vocal and a backing track; confirm which track is the lead vocal; check source availability, clipping and level; set Sa and octave. A simple, linear flow for the vocalist; consider a **simplified "Vocalist" workspace** (a large waveform, the pitch view, one record button) alongside the full arrange window.
- **Cloud assistant opt-in:** shows the **provider, the endpoint, the data categories sent (text only, never audio) and the retention terms**; a **budget** in currency to set before first use; and the credential stored in the system keychain. It must state that the user's own key does not remove charges or data-retention concerns.
- **Empty states, errors and loading** for each panel.
- **Missing media:** a clear relink flow; silence is never substituted.

## 6. Icons

An original icon set, **20 px and 16 px**, on a consistent grid (state the live area, padding and **stroke width, for example 1.5 px**, round or square caps and joins). Needed: all transport and edit tools in §5.1, record arm, monitor, solo, mute, bypass, insert, send, automation, loop, grid, zoom, pitch, assistant, audition (A/B), keep, discard, undo, warning, error, info, locked, hidden, folder, and settings. **Provide each as an SVG built from simple primitives** (lines, arcs, rounded rectangles, circles) so a developer can redraw it in code; no raster, no complex path effects. Active and inactive variants.

## 7. Deliverables and format

For each round, give me:

1. **Token sheet**: a table of name, value (hex or px), usage, and for colours the **contrast ratio** against its usual background.
2. **Component sheet**: every component in §5.2 with **all states side by side** and **exact dimensions** (px) and spacing.
3. **Full-screen mockups** at 1440 × 900 (dark; light where asked), with a **redline** (measurements) for the track header, the clip and the transport.
4. **Icon sheet**: each icon at both sizes, plus SVG source.
5. **Notes** where a behaviour matters (what is clickable, what happens on hover, what a disabled control explains).
6. **Microcopy**: the exact wording for labels, empty states, warnings and errors in §5.5 to §5.9. Plain language, no jargon, no marketing words such as "AI-powered", "smart" or "magic".

## 8. Hard rules

- **No imitation.** Do not reproduce another product's interface, icons, colours as a set, or artwork (for example Logic Pro, Pro Tools, Ableton Live). You may use general qualities such as clear hierarchy, restraint and consistency. The result must be original and not mistaken for another product.
- **Only open or original assets.** Fonts must be SIL OFL, Apache-2.0 or similar, with the licence stated. No stock imagery. Icons are original.
- **No dark patterns.** Never push "Keep" over the original; never hide the cost or data use of the cloud feature.
- **No raw technical output** in the main flow: no JSON, IDs only where a user needs to tell two things with the same name apart.

## 9. Implementation constraints

The interface is drawn with **egui**, an immediate-mode Rust toolkit, so **design within what it can do cleanly**:

- Filled and rounded rectangles, strokes, polylines, circles, simple **vertical gradients**, and text. Custom-painted waveforms, meters and pitch contours are fine.
- **No blur, no backdrop blur, no glass effects, no soft inner shadows.** Depth comes from layer colour, 1 px borders and a single offset translucent shadow.
- **No complex animation.** Hover, press and panel transitions up to 200 ms; meters with attack and decay.
- Text is a single UI family plus a monospace or tabular numeric style.
- Sizes in whole logical pixels on a 4 px grid; the app scales for Retina.
- **Panels dock and resize**, so give minimum and maximum sizes for each panel.

## 10. Reference

`soundcraft-current-ui.png`: the Arrange window today with a demo session (nine tracks, drum and synth parts). Treat it as the "before". Keep its structure (menu, toolbar, track list, ruler, track rows, clip list, bottom tabs, status line) unless a better structure serves the same functions.
