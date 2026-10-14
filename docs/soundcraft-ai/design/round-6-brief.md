# Round 6 brief for Claude Design (follow-up)

**Version:** 0.1 · **Date:** 15 October 2026 · **For:** the design tool, in the same project as rounds 1 to 5. **Reviewed by:** you, before sending.
**Builds on:** the Round 1 to 5 bundle. Nothing already designed is to be redone.

## 0. What this round is

Rounds 1 to 5 delivered the Arrange window, the design system, the new screens and the notes and copy. They were good, and the direction is settled: **Graphite, dark and light, IBM Plex Sans and Mono, the tokens in the design system sheet.**

Five things are missing or need correcting. Please add them as **new boards numbered 6a to 6f** and **extend the Design System sheet** where noted. Use the existing token names only; if you need a token that does not exist, list it in a short "new tokens" table instead of inventing values inline.

| Board | What |
|---|---|
| **6a** | The change review for an **explicit assignment** (a variant of 3c), dark and light |
| **6b** | **Messages** for the situations in section 2 (written copy, plus where each appears) |
| **6c** | **Icon set**, 20 px and 16 px, as SVG |
| **6d** | **Component state sheets** that are missing (section 5) |
| **6e** | **Fader and track-button redraw**: the real fader scale, and hit areas for the dense controls |
| **6f** | Updated **Design System sheet**: the new components, the new icons, any new tokens |

## 1. What has been decided, so please do not redesign it

**The approval flow (decision UI-1).** There are two paths, and the app, not the user or the interface, decides which one a plan takes:

- **A production proposal** (generated from a goal such as headroom, vocal clarity or even level): the change review (3c) shows **Preview** (primary) and **Reject**, and **no Approve**. Preview renders the candidate and opens the audition (3d), where **Keep this change** is the single approval. **Keep this change, Keep the original and Discard stay equal in size and weight**, and keyboard focus starts on the A/B switch.
- **An explicit assignment** (the user states an exact value or name, for example "set Kick to −6 dB" or "rename this track"): the change review shows **Approve** (primary) and **Reject**. There is no render and no audition.
- Editing any value invalidates a preview and any approval. A plan made stale by a session change cannot be approved on either path.

Everything in the existing boards stays as it is, except the change review actions (3c): please redraw 3c's production-proposal state with **Preview · Reject** only, as part of 6a.

## 2. Messages (board 6b)

Write plain-language messages for the situations below. The app already knows each situation; you write what the user reads. Please give, for each, a **message (at most two sentences)** and a **"what you can do" line**, and say **where it appears**: in the assistant panel's response, as a banner in the change review, or both.

**Style rules**, from the existing copy: plain words, no jargon and no blame ("you did X wrong"). **Always say whether anything was changed** (usually "Nothing was changed."). Never "error", "invalid", "AI-powered", "smart", "magic" or "better". Never show a score as a percentage. Sample numbers are fine. Match the voice of the existing "Cannot propose" and "No change" strings.

| # | Situation | Facts the message can use | What the user can do |
|---|---|---|---|
| 1 | **Stale measurements.** The session changed after the vocal was measured | The proposal was based on an earlier version of the track | Run the vocal check again |
| 2 | **The track is not confirmed as the lead vocal** | Roles are set by the user, never guessed from a name | Confirm which track is the lead vocal |
| 3 | **The fader is automated.** A fixed change would be overridden by the automation | The track's level follows an automation curve | Set the track's automation mode to Off, or lower the level by hand |
| 4 | **Too little signal.** Less than 1 second of sound in the selected range | A range needs at least 1 s of signal to set a level | Select a longer part that includes the singing |
| 5 | **Almost silent.** The loudest point in the range is below −60 dBTP | Sample: "−72 dBTP" | Select where the vocal is singing, or check the track's input |
| 6 | **The source is clipped, and the change would lower the level** (a note on a proposal, not a refusal) | Lowering the level gives headroom but does not repair clipping | Nothing needed; it is information |
| 7 | **The fader is already at its limit** (+6 dB), so there is no useful change to propose | The proposal would need a level above the usable range | Lower the level by hand, or choose another track |
| 8 | **The fader is far outside the usable range** (below −60 dB), so a change cannot be made safely | The fader is at, for example, −100 dB | Move the fader into range first |
| 9 | **A limit was applied to a proposal** (two lines: the change is capped at 12 dB, and the fader is kept within −60 to +6 dB) | Samples: "Limited to 12 dB per change." exists already; the fader-range one is new | Information |
| 10 | **A goal the app does not support yet** | The supported goals today are Headroom, Vocal clarity and Even level | Choose one of the goal chips |
| 11 | **Something went wrong preparing the proposal** (a catch-all for problems that are not the user's doing) | There is a short technical code the user may need to give support | Try again; the code sits under Details |

**Also confirm or redraw these three strings** (proposed in decision UI-1; keep the voice, change the words freely):

- "A plug-in in the signal path cannot be rendered here." (why Preview is off)
- "The session changed. Ask again to review this change." (an assignment gone stale)
- "You kept the original. Nothing changed." (history entry)

**Format for 6b:** a table I can drop into code with a `key` in snake_case (for example `stale_measurements`), the `text`, the `what_you_can_do` line, and `shown_in`.

## 3. The explicit-assignment change review (board 6a)

A variant of the change review (3c), **440 wide (min 400)**, dark and light. States: **ready** (Approve primary, Reject), **stale** (Approve off, with a message), and **unavailable**. Show a **rename** and a **level assignment** ("set to −6.0 dB, was −3.0 dB"). Keep the same structure as 3c (title, "Not applied" badge, the change, before and after with units, the target with its name and ID) but with **no preview, no basis block from a measurement and no audition**, since nothing was measured. Also redraw the production-proposal state of 3c with **Preview · Reject**.

## 4. Icon set (board 6c)

Original icons, **20 px and 16 px**, on one consistent grid. Please state the live area, padding, **stroke width (1.5 px suggested)**, cap and join style, and corner radius. **SVG built from simple primitives** (lines, arcs, rounded rectangles, circles) so a developer can redraw each in code; no raster, no effects. Provide **active and inactive** versions, and show each at both sizes on both themes.

**Needed (about 60):**

- **Transport:** play, stop, pause, record, rewind, fast forward, return to start, loop.
- **Edit tools:** zoom, trim, selector, grabber, scrubber, pencil, smart tool.
- **Edit modes and view:** shuffle, spot, slip, grid; zoom in, zoom out, zoom to selection; waveform view, notes view.
- **Track:** record arm, input monitor, solo, mute, bypass (power), insert, send, automation (read, write, touch, latch), lock, hidden, folder, inactive, instrument, aux.
- **Product:** assistant, vocal pitch, A/B audition, keep, discard, reject, undo, redo, preview, stale (out of date), recovery.
- **Status:** warning, error, info, success, cancel, settings, search, close, overflow menu, disclosure arrow (open and closed).

**Rule that matters:** the icons must be **original**. Do not reproduce the glyphs of Pro Tools, Logic Pro or any other product, in particular the transport, smart-tool, grabber, scrubber and automation symbols, which are the most imitated. Where convention strongly suggests a shape (a triangle for play, a circle for record), keep it generic.

## 5. Component state sheets still missing (board 6d)

For each, show **default, hover, pressed, focus and disabled** where they apply, in both themes, with **exact dimensions**:

| Component | States and notes |
|---|---|
| **Dropdown** | closed, open (with a selected item and a disabled item), with a unit |
| **Slider** | default, at a value, dragging with a tooltip, disabled |
| **Context menu** | with a submenu, a disabled item, a checked item, a shortcut column |
| **Tooltip** | normal, and the "why this is off" form used for disabled controls |
| **Dialog** | with a title, body and two or three actions; and the **disk-full dialog** that takes focus (from board 3f) |
| **Toast and banner** | info, warning, error, success; with and without an action; the banner levels used in the change review |
| **Progress** | determinate, indeterminate, **cancelling** ("Stopping… Nothing was changed.") |
| **Collapsible section and panel header** | open, closed, with a badge |
| **Level meter** | mono and stereo; peak and RMS; the latched clip box (set and cleared); peak hold (1.5 s); the scale with its labels; the empty well for an off track; both themes |
| **Scroll bar** | idle, hover, dragging |
| **Text field** | with a unit, invalid (with its message), read-only |

## 6. The fader scale and the dense controls (board 6e)

**The fader.** The mockup's scale was a placeholder: the board labelled "0 dB at 91 px", but its own value formula gave about −22.5 dB there. Please **redraw the fader scale to the app's real law**. The fader runs from −∞ (bottom) to **+12 dB** (top). Positions as a fraction of the travel from the bottom:

| dB | +12 | +6 | 0 | −6 | −12 | −18 | −24 | −36 | −48 | −60 | −72 | −96 | −∞ |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Fraction of travel | 1.000 | 0.875 | 0.750 | 0.625 | 0.500 | 0.375 | 0.250 | 0.196 | 0.165 | 0.138 | 0.114 | 0.072 | 0.000 |

(So the top three quarters are an even scale from −24 to +12, and the bottom quarter compresses everything below −24.) Choose which marks to label at each fader height, and show the **unity (0 dB) mark** clearly. Keep the cap, states and the tooltip as designed.

**Dense controls and hit areas.** The track buttons are drawn 22 × 18 (28 × 22 in the inspector), smaller than a comfortable target. Please show, for the track buttons and the other dense controls, **the visual size and the hit area separately**: the hit area may be larger than what is drawn (for example by padding into the gap between neighbours) as long as neighbours do not overlap. State the hit area for each and the minimum gap. Show how keyboard focus looks for these small controls.

## 7. Format and rules

- Same format as the first brief: a **token sheet** for anything new (name, value, usage, contrast), **component sheets with all states side by side and exact px**, **mockups at 1440 × 900** where a full screen is shown, and **microcopy as a table**.
- **Both themes** for everything. **State never by colour alone.**
- **No imitation** of another product's interface or icons. **Only open or original assets**; the fonts stay IBM Plex.
- **egui limits** as before: filled and rounded rectangles, strokes, polylines, circles, vertical gradients, a 1 px inset highlight, the offset shadow, text. No blur, no glass, no complex animation (200 ms at most).
- Please **list anything in rounds 1 to 5 that you now think was wrong** or that conflicts with this round, rather than changing it silently.

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 15 October 2026 | Follow-up brief: messages, explicit-assignment review, icons, missing component states, real fader scale and hit areas. |
