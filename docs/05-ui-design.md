# 05 · Visual identity and the three screens

Status: design proposal. Everything visual here is a design decision (see D-UI items in [01 §3](01-overview-and-architecture.md#3-design-decisions)). The accessibility numbers are requirements we hold ourselves to and are taken from WCAG 2.2 ([refs](09-references.md#accessibility-and-rendering)).

## 1. Identity in one paragraph

Stillkey looks like polished black glass with one light source above it. Surfaces are near-black and cool. Panels are slightly lighter than the desktop, have a one-pixel highlight on their top edge and a one-pixel shadow under their bottom edge, and carry a faint top-to-bottom gradient. Cyan is the only accent and it always means the same thing: *this is where you are* (focus, selection, the trusted system voice). Red and amber never decorate. Red means something failed or was revoked. Amber means a decision or a risk needs your attention. Everything else is greyscale.

Three rules keep the gloss from hurting legibility:

1. **Text never sits on a reflection.** Under any glyph, gradients and highlights keep the flattened background within 6 lightness points (HSL L) of the surface token: on `panel`, no lighter than `#1C222B`. The 1 px top-edge highlight (`#2C3A4C`) is exempt, and no glyph is drawn within 4 px of a panel edge. Translucency has its own limit, the contrast rule in §6.
2. **Colour is never the only signal.** Every state has a word and a glyph next to its colour.
3. **Motion is optional.** The interface is complete with zero animation; motion is added later as an enhancement and can be turned off.

## 2. Design tokens

### 2.1 Colour

Contrast ratios were computed with the WCAG 2.2 relative-luminance formula against the flat surface tokens (script result recorded during design; re-run it when a token changes). Stage 2 translucency is bound by the contrast rule in §6.

| Token | Hex | Use | Contrast on `panel` | Contrast on `select` |
|---|---|---|---|---|
| `void` | `#06080C` | Desktop base, dimmed backdrop | – | – |
| `base` | `#0A0D12` | Top bar, shelf | – | – |
| `panel` | `#10151D` | Panels, tables | – | – |
| `raised` | `#18202B` | Buttons, headers, hovered row | – | – |
| `select` | `#1B2A38` | Selected row, active tab | – | – |
| `text` | `#E9EEF5` | Primary text | 15.70 | 12.55 |
| `text-2` | `#AAB6C5` | Secondary text, column headers | 8.90 | 7.11 |
| `muted` | `#8492A6` | Hints, disabled labels (still readable) | 5.79 | 4.63 |
| `cyan` | `#3DDCFF` | Focus ring, selection bar, trusted chrome | 11.23 | 8.98 |
| `cyan-deep` | `#12A5CC` | Cyan fills behind dark text | 6.35 | 5.07 |
| `red` | `#FF6B74` | Errors, crashes, revoked | 6.63 | 5.30 |
| `amber` | `#FFB938` | Warnings, requests waiting for a decision | 10.68 | 8.54 |
| `border` | `#2A3646` | Decorative hairlines only (1.49:1, not relied on) | 1.49 | 1.19 |
| `border-strong` | `#6B7F99` | Boundaries the user must see: inputs, buttons, inactive focus | 4.46 | 3.57 |
| `danger-bg` | `#2A1216` | Background tint behind red text | red 6.36 | – |
| `warn-bg` | `#2A2010` | Background tint behind amber text | amber 9.33 | – |

Targets: body text ≥ 7:1 (we exceed WCAG 1.4.6 AAA for primary and secondary text), any text ≥ 4.5:1 (1.4.3 AA), focus indicators and required boundaries ≥ 3:1 (1.4.11 AA). `void` text on `cyan`, `amber` and `red` fills is 12.3:1, 11.7:1 and 7.3:1, so filled buttons and the trusted band use `void` text. `void` on `cyan-deep` is 6.95:1 (AA).

A **Contrast+** theme shares all layout and changes only colour. It maps `panel`, `raised`, `select`, `danger-bg` and `warn-bg` to `#000000`, `text` to `#FFFFFF`, and `text-2` and `muted` to `#AAB6C5` (10.21:1 on black), and draws every boundary in `border-strong`. Selection is the 4 px `cyan` bar plus a 1 px `border-strong` outline around the row, and remembered focus is a dashed 1 px outline, so the two stay apart when every boundary has the same colour. There is no gradient, dither or highlight, and the backdrop under a modal is flat `#000000`. The boot option `ui.contrast=high` starts in Contrast+ and `Ctrl+F12` switches it at run time; the run-time choice is not remembered until storage exists.

Two more boot options exist: `ui.scale=2` (every cell and spacing value doubled, for high-density displays; anything without a native double size is drawn with each pixel doubled; it is not part of any Milestone 4 pass criterion) and, from stage 2, `ui.transparency=reduced` (opaque panels, no backdrop sampling). Boot options reach `desktop` through `init`, which receives the command line from the kernel and forwards the `ui.*` options ([03 §7](03-kernel-and-interfaces.md#7-user-space-pieces-we-write)); no app can read them.

### 2.2 Type

| Stage | Faces | Sizes | Notes |
|---|---|---|---|
| 1 (first framebuffer UI) | One monospaced bitmap face, three cell sizes: 8×16 body, 12×24 heading, 16×32 title | Integer cells only | No anti-aliasing. Glyphs are 1-bit masks drawn in a token colour. Font: Spleen 2.2.0 (BSD 2-Clause). It ships six sizes (5×8, 6×12, 8×16, 12×24, 16×32, 32×64); we use 8×16, 12×24 and 16×32, and 32×64 as the doubled title (with `ui.scale=2` the body is the native 16×32 and the heading is 12×24 with each pixel doubled). The 8×16 size and the licence file arrive with the kernel console in Milestone 2 (stage 0 in §6), the larger sizes in Milestone 4. The licence text goes in the image. |
| 2 (polish) | UI sans for labels, mono for numbers and paths | 14, 16, 20, 28 px | Pre-rasterized 8-bit alpha atlases baked at build time by `xtask`. Proposed: Inter + JetBrains Mono (OFL 1.1, no reserved font name, so shipping pre-rasterized atlases under the original names is permitted). IBM Plex is an alternative, but its licence reserves the name "Plex", so a converted build would have to be renamed (D-UI-2). The OFL text and each font's copyright line ship in the image next to the atlases. |
| 3 (mature) | Same faces, runtime rasterizer | Any | Scale factors 1× and 2×, user text-size setting. |

Rules: numbers are always monospaced and right-aligned in tables; units are separated by one cell space in stage 1 and a thin space from stage 2, and use binary prefixes (`KiB`, `MiB`); paths are monospaced; nothing is set smaller than a 16 px cell in stage 1 or 14 px in stage 2; sentence case everywhere; no all-caps body text.

### 2.3 Space, shape, depth

- 8 px grid. Panel padding 16 px. Table rows 28 px (16 px text + 6 px above and below); lines in the Workbench detail pane use the same 28 px row. Top bar 32 px. Shelf 40 px.
- Corner radius: 0 in stage 1 (square, with the 1 px highlight carrying the "glass" feel), 8 px panels and 6 px controls from stage 2.
- Depth has three levels only: desktop, panel, modal. A modal dims everything below it except the top bar (§3.3) by multiplying each stored channel value by 0.45 through a 256-entry table. In stage 1 the chrome that `desktop` draws uses pre-computed dimmed tokens and app surfaces go through the table when composited; stage 2 applies the same table after blending.
- Focus: 2 px `cyan` outline drawn outside the control with a 1 px `void` gap, so it meets 3:1 against both the control and its surroundings. When a pane is inactive its remembered focus is drawn as a 1 px `border-strong` outline so you can see where Tab will land.
- Selection (a row you are on) is a 4 px `cyan` bar at the row's left edge plus the `select` background. Selection and focus are different marks and can coexist.

### 2.4 State vocabulary

| State | Colour | Glyph | Word shown |
|---|---|---|---|
| Running (alive) | `text` | filled circle | Running |
| Exited normally | `muted` | dash | Exited |
| Ended by you | `text-2` | filled square | Ended |
| Crashed | `red` on `danger-bg` | cross | Crashed |
| Needs decision | `amber` on `warn-bg` | triangle with `!` | Asks |
| Request denied | `text-2` | barred circle | Denied |
| Access active | `text` | key | e.g. "Read" |
| Access revoked | `red` | key with slash | Revoked |
| Access released | `muted` | key outline | Released |
| System (trusted) | `text-2` | shield | System |

A process's state is its lifecycle, not what the scheduler is doing with it: Running (alive, whether its threads are ready, running or waiting), Asks (alive with a request waiting for an answer), Crashed, Exited, Ended. The scheduler's wait reason appears only in the Workbench detail pane, as a phrase ("Waiting for a key", "Sleeping", "Waiting for a message", "Waiting for your decision").

An active grant is drawn in `text`, not `cyan`: cyan stays reserved for focus, selection and the trusted band. A grant is Released when its last handle is closed: the app closed it, or the app is gone (exited, crashed or ended).

Spleen supplies the filled circle, dash and cross. The key, key with slash, key outline, shield, warning triangle, square and barred circle are our own 1-bit icons, drawn at 8×16, 12×24 and 16×32 and kept in `assets/icons`.

## 3. Screens

Reference canvas is 1280×800 (the size we expect from OVMF's default display; assumption A-07). Layouts are defined in grid units and reflow at 1920×1080 by widening panels, never by scaling bitmap glyphs fractionally.

There is no wall clock until a time source is added (decision D-UI-7). Every time on screen is time since boot: the top bar shows `up 02:14`, a grant `granted at +02:11 by you`, an event `+02:11 Granted …`. The form is minutes and seconds, with hours in front after the first hour (`up 1:02:14`).

The wireframes are 98 columns wide for a screen of 160 cells, so they show structure and wording, not exact proportions. In them `||` is the selection bar, `o` a filled circle, `x` the cross, `>` the key glyph and `[[ ]]` the focused control.

### 3.1 Desktop and app launcher

```
+------------------------------------------------------------------------------------------------+
| Stillkey   Home             2 running   0 grants active   0 crashes   up 02:14   F12 Workbench |
+------------------------------------------------------------------------------------------------+
|                                                                                                |
|           +------------------------------- Launcher -------------------------------+           |
|           | Find an app: _                                                         |           |
|           +------------------------------------------------------------------------+           |
|           || Notes       Shows one text file                            No access  |           |
|           |  Ticker      Counts once a second                           No access  |           |
|           |  Faulter     Crashes on purpose                             No access  |           |
|           |  Workbench   See what apps do and reach                     System     |           |
|           +------------------------------------------------------------------------+           |
|           | Up/Down select   Enter launch   Right details   Tab shelf   Esc clear  |           |
|           +------------------------------------------------------------------------+           |
|                                                                                                |
+------------------------------------------------------------------------------------------------+
| Running   (o) Workbench   (o) Ticker                                F9 next app   F11 launcher |
+------------------------------------------------------------------------------------------------+
```

- **Top bar** (the first row): product name, workspace name, live counters, time since boot, and a permanent hint for the Still key. The counters are running apps; active grants, which `desktop` counts through its own `Diag` handle, so the number does not depend on the Workbench ([04 §7](04-security-and-permissions.md#7-workbench-diagnostics-interface)); crashes since boot, red with the word "crashes" when non-zero; and, only when there are any, requests waiting for an answer, amber with the word "asking". While a file-access request is open the top bar becomes the trusted band (§3.3).
- **Launcher**: a centred glass panel, 640 px wide. A text field filters the list as you type. Each row is name, one-line description, and an **access summary** that `desktop` reads through its own `Diag` handle: "No access", "1 file", or "System". A running app shows its current grants; an app that is not running shows "No access", because grants end with the process.
- **Details (`Right`)**: expands the selected row to show the app's identity (bundle name, image digest prefix), the requests declared in its launcher-registry entry (information only; declaring grants nothing), and its last crash if any. `Left` closes it.
- **Shelf** (the last row): one tile per running app, in launch order, and at its right end the hints for `F9` and `F11`. Focused app has a cyan underline. A tile whose app has a request waiting shows the warning triangle and the word "Asks". A crashed app's tile stays, in red with the word "Crashed", until it is dismissed with `Delete`.
- **App windows**: in the demo an app occupies the central region between the top bar and the shelf (1280×728 px on the reference canvas; one visible app at a time, `F9` switches). `desktop` creates each app's surface and sends the app a handle to map it; an app never supplies memory to `desktop` and never receives the framebuffer. Free-floating overlapping windows are a later step.

Keys: typing filters; `Up/Down` move in the list; `Enter` launches, or switches to the app if it is already running; `Right` opens details for the selected row and `Left` closes them; `Tab`/`Shift+Tab` move between launcher and shelf; on the shelf `Left/Right` move between tiles, `Enter` switches to that app, `Delete` dismisses a crashed tile; `Esc` clears the filter; `F11` shows the launcher from anywhere; `F9` cycles apps (`Alt+Tab` too where the host passes it through); `F12` (the Still key) opens the Workbench from anywhere. `F9`, `F11` and `F12` are reserved by `desktop` (decisions D-UI-3 and D-UI-6).

### 3.2 Workbench process inspector

One screen answers both questions. The table says what is running. The detail pane says what the selected app is doing (top block) and what it can access (middle block), with its crashes below. The Workbench takes the whole screen: its header replaces the top bar and its key line replaces the shelf. It opens with the visible app selected; when no app is visible it keeps its last selection.

```
+------------------------------------------------------------------------------------------------+
| Workbench    6 processes    19.4 MiB of 256 MiB    1 grant active    1 crash       Esc Desktop |
+-------------------------------------------------------+----------------------------------------+
|  State      Name       PID    Memory  CPU   Access    | Notes                    PID 7     App |
+-------------------------------------------------------+----------------------------------------+
|  o Running  init         1    96 KiB   0%   System    | WHAT IT IS DOING                       |
|  o Running  permd        2   212 KiB   0%   System    |   State     Waiting for a key          |
|  o Running  desktop      3  15.6 MiB   3%   System    |   Running   2 min 14 s   1 thread      |
|  o Running  workbench    4   5.0 MiB   1%   System    |   Memory    4.0 MiB                    |
|| o Running  Notes        7   4.0 MiB   0%   1 file    |     image 96  heap 252  stack 64 KiB   |
|  o Running  Ticker       8   3.7 MiB   0%   No access |     tables 20 KiB  shared 3.6 MiB      |
|  x Crashed  Faulter      9         –    –   No access |                                        |
|                                                       | WHAT IT CAN ACCESS                     |
|                                                       |  > Read   /docs/welcome.txt [ Revoke ] |
|                                                       |           granted at +02:11 by you     |
|                                                       |    Window  its own window only         |
|                                                       |    Keys    only while focused          |
|                                                       |                                        |
|                                                       | RECENT CRASHES                         |
|                                                       |    None                                |
+-------------------------------------------------------+----------------------------------------+
| +03:02 Crashed  Faulter  page fault at 0      +02:11 Granted  Notes  Read /docs/welcome.txt    |
+------------------------------------------------------------------------------------------------+
| Up/Down process   Tab pane   Enter open   R revoke   K end   S sort   / find   Esc desktop     |
+------------------------------------------------------------------------------------------------+
```

- **Header**: totals that should be glanceable from across the room: process count, memory used of total, active grants, crashes (red when non-zero) and, only when there are any, requests waiting for an answer ("1 asking", amber).
- **Process table** (left, about 58% width): State (glyph + word; the lifecycle of §2.4), Name, PID, Memory (right-aligned; the five counters of the detail pane added up, so a shared surface is counted under each process that maps it and the column does not add up to the header's total), CPU (share of the last second), Access (count of active grants, "No access", or "System" for trusted services). Sorted by launch order by default so rows do not jump; `S` cycles sort. Crashed, exited and ended rows keep their last identity and show `–` for live values. A PID is never reused within a boot, so a row, a grant and a crash record with the same PID always mean the same process.
- **Detail pane** (right):
  - *What it is doing*: the kernel's wait reason mapped to a phrase (§2.4), uptime, thread count, and memory with the five counters from [04 §7](04-security-and-permissions.md#7-workbench-diagnostics-interface): image, heap, stack, page tables and shared. An app's window surface is shared memory that `desktop` created, so it shows here but counts against `desktop`'s memory limit, not the app's.
  - *What it can access*: first any request waiting for an answer, on one line (the warning triangle, "Asks", the action and path, and the hint "answer on the prompt (Esc)"), then every grant as **action + object + when + who decided**. A grant takes two lines: action, path and the Revoke control, then when and who. Paths wrap, never truncate. Below the revocable grants are the fixed endowments every app gets (its own window, keys while focused), listed so that nothing an app can do is invisible.
  - *Recent crashes*: time, plain-language reason, and `C` for the crash sheet. By default the sheet is a **summary**: the plain-language reason, fault kind, faulting address, and instruction pointer relative to the app image. It shows no CPU registers, because a register can hold the app's data. Raw registers appear only in **debug mode** (boot option `debug=registers`): the top bar then reads "Debug: registers" in `amber` for the whole session, and the sheet gains a Registers section below the summary. Never memory contents, and never registers for a System-class process, in either mode. The rule and its reason are in [04 §6](04-security-and-permissions.md#6-crash-records).
- **Requests waiting for an answer** come from `permd`'s own list, never from anything an app drew. The header counts them, the asking process's row reads "Asks", and its detail pane shows the request first. A request cannot be answered here.
- **Event strip**: the last audit events (asked, granted, denied, revoked, released, crashed, ended), newest left.
- **Revoke flow**: `R` works from the table or the grant list. From the table it moves focus to the selected process's first active grant; if that process has exactly one active grant the confirmation opens at once, otherwise you pick a grant and press `R` or `Enter`. An inline confirmation replaces the grant's two lines: "Stop Notes reading /docs/welcome.txt? Notes keeps what it has already read. `[ Revoke ]` `[ Keep ]`", focus on Revoke because you just asked for it; `Esc` keeps. When the confirmation closes, focus returns to where `R` was pressed, so after `R`, `Enter` from the table one `Esc` leaves the Workbench. The confirmation always names the app and the full path. On confirm the row turns to "Revoked +03:40" in red and stays visible for as long as the process is listed; when the process later exits, crashes or is ended the row adds "Released" and the time.
- **System rows** are inspectable but cannot be ended or revoked from here; the controls are absent, not disabled.

Keys: `Up/Down`, `PageUp/PageDown`, `Home/End` (first and last row; the table is navigated by row, as a list) in the table; `Tab`/`Shift+Tab` between table, grants, crashes; `Enter` opens the selected row's detail pane or activates the focused control; `R` revoke; `K` end process (confirmation, focus on Keep running); `C` crash sheet; `S` sort; `/` find by name; `Esc` back one level, then to the desktop. While find is open every printable key is text and `Esc` closes it; `R`, `K`, `C` and `S` act only when the table or a grant has focus. The key line shows `R revoke` only when the selected process has an active grant, `K end` only for an app, and `C crash` only when the selected process has a crash record.

### 3.3 File-access request

A modal sheet drawn by the trusted desktop on a layer apps cannot draw to. It confirms a path the app named; that is the design for Milestones 3–4 (decision D-S-5), and a file chooser drawn by `desktop`, where your choice is the grant and the app names no path, is the planned replacement ([04 §10](04-security-and-permissions.md#10-open-questions)).

A request is **waiting** from the moment `permd` accepts it until it is answered or cancelled, and **open** while its sheet is on screen; the sheet then receives every key except the Still key and `Ctrl+F12`. While the Workbench covers the sheet, or while the request's app is not the visible app, the request is simply waiting. The Workbench, while it is in front, has the keys, and a request cannot be answered there. Leaving the Workbench re-opens the sheet with focus on Deny and the input guard described below. If the requester exits, crashes or is ended, the request is cancelled, logged as denied (`actor=permd reason=requester-exit`) and the sheet closes.

```
+------------------------------------------------------------------------------------------------+
| [key] Stillkey asks · Notes                                           1 asking   F12 Workbench |
+------------------------------------------------------------------------------------------------+
|   (Notes' window, dimmed)                                                                      |
|            +----------------------------------------------------------------------+            |
|            | [key] Stillkey asks                                    F12 Workbench |            |
|            +----------------------------------------------------------------------+            |
|            |                                                                      |            |
|            |   Notes wants to read one file                                       |            |
|            |                                                                      |            |
|            |   App       Notes            PID 7 · untrusted app                   |            |
|            |   File      /docs/welcome.txt                                        |            |
|            |             read-only test volume                                    |            |
|            |   Action    Read                                                     |            |
|            |                                                                      |            |
|            |   If you allow this, Notes can read this file until you revoke it    |            |
|            |   in the Workbench or Notes closes. It cannot change the file or     |            |
|            |   see other files. Revoking stops further reading; it cannot take    |            |
|            |   back what Notes has already read.                                  |            |
|            |                                                                      |            |
|            |             [[ Deny ]]                 [ Allow read ]                |            |
|            |                                                                      |            |
|            |   Left/Right or Tab choose   Enter or Space confirm   Esc deny       |            |
|            +----------------------------------------------------------------------+            |
|                                                                                                |
+------------------------------------------------------------------------------------------------+
| Running   (o) Workbench   (o) Notes   (dimmed)                      F9 next app   F11 launcher |
+------------------------------------------------------------------------------------------------+
```

The first row is the top bar in its trusted-band form, and the sheet's own first row is the same band: `cyan` fill, `void` text, key glyph.

- **Three labelled facts, always in this order**: App, File, Action. The app name comes from the launcher's registry, not from the app. The path is the canonical full path resolved by the broker; long paths wrap, never truncate in the middle. The action is one verb from a fixed list (`Read` only until writable storage exists).
- **No file facts before the decision**: the sheet is shown for any well-formed path, whether or not the file exists, and never shows a size or anything else about the file; the second File line names only the volume. `permd` checks that the file exists only after the decision, so neither the status nor the time to an answer tells an app whether a path exists ([04 §4.1](04-security-and-permissions.md#41-behaviour-case-by-case)).
- **The consequence in plain words**: scope, duration and limits, including that the grant ends when the app closes and that revoking cannot take back what was already read.
- **Buttons are verbs**: "Deny" and "Allow read". Initial focus is on Deny. Each button is its own Tab stop, `Left/Right` also move between them, and `Enter` or `Space` activates the focused one. `Esc` denies. There is no timeout, no "always allow", and no pre-selected Allow.
- **Trusted chrome**: an app can draw an identical sheet inside its own surface, and that gains it nothing, because only `desktop` can answer `permd`; keys typed into an imitation grant nothing. The sheet is therefore not the trust signal. The trust signal is the top bar, which only `desktop` draws and which is composited above every App-class surface; only the Workbench, a System-class program that takes the whole screen, covers it. While a real request is open the top bar itself becomes the trusted band, with the same fill and key glyph as the sheet's band, reading "Stillkey asks · Notes", and the shelf is dimmed with everything else. The Still key opens the Workbench, which lists the requests that are really waiting. The Milestone 4 test uses the sample `imposter`, which fills its surface with `#FF00FF`: the screenshot must have no `#FF00FF` pixel inside the top bar, the shelf or the real sheet's rectangle. `imposter` is not in the launcher; `init` starts it when the boot option `test=m4-imposter` is given.
- **Input guard and queueing**: one request at a time. Every time a sheet appears (first request, next in the queue, or re-shown after the Workbench) `desktop` discards queued key events, ignores all keys for 500 ms, ignores auto-repeat, and ignores any key that was already down until it is released. The Still key is exempt. Focus always starts on Deny. A request from an app that is not the visible app does not open a sheet: it waits, counted as "asking" in the top bar, until you switch to that app. An app therefore cannot time or stack prompts to catch a key meant for something else. `desktop` writes `SK:DESKTOP:PROMPT app=… path=… action=…` with `debug_write` every time it shows a request's sheet (first appearance, next in the queue, or re-shown after the Workbench), so a scripted test waits out the guard after that line before it sends a key. The line reaches the serial log as the text of `desktop`'s `SK:APP:OUT` line.
- **Result**: a non-modal strip at the bottom of the app area for 4 s (or until the next key press in reduced-motion mode). After Allow: "Notes can now read /docs/welcome.txt · F12 to manage". After Deny: "Notes was not given /docs/welcome.txt". After Allow for a path that does not exist: "/docs/x.txt does not exist; nothing was shared"; the app receives `ACCESS_DENIED`, the same status as after Deny.

### 3.4 Assistant (stage 4b)

The assistant is an app with one surface: a conversation, newest at the bottom, and a line to type in. Each thing it does for the person is one row of the conversation in `text-2` ("Started Notes", "Read the outline of Notes", "Typed 12 keys in Notes"), so what was done can be followed without opening the Workbench. The rules behind this section are in [04 §11](04-security-and-permissions.md#11-delegation-to-an-assistant-stage-4b).

- **Launcher row.** The access summary reads "No access · Outside", or the count of its grants and "Outside". The word Outside is drawn in `amber` and is never shortened.
- **The list sheet.** The request sheet of 3.3 with a list in place of the one path: a row for each delegation asked for ("Start Notes", "See Notes", "Type in Notes", "Read /docs/welcome.txt"), each with a tick. `Up` and `Down` move through the rows, `Space` removes or sets a tick, `Tab` goes on to Deny and Allow. Focus starts on Deny. Under the list, in `amber`: "What Assistant can read or see it may send to the model outside this machine." Allow with no tick left is a Deny.
- **The access sheet (`F8`).** The same sheet, opened by the person while the assistant is in front. It lists every app of the launcher with its four rights and every file the assistant has asked about, with a tick where a grant is active. Setting a tick grants; removing one revokes. `desktop` draws it, like every sheet.
- **Top bar.** While a Start, Type or End delegation is active the top bar reads "Assistant controls Notes" in `amber`, with the names of at most two apps and then a count. See and Read alone do not show it; they are in the count of active grants.
- **Hands off (`F10`).** Revokes every delegation at once and shows "Hands off: 3 grants ended" in the top bar for two seconds. It works on every screen, also while a sheet is open.
- **Workbench.** Delegations are listed under the assistant like file grants: "Type in Notes · 41 uses · granted at +03:02 by you". `R` revokes one. The assistant's row carries the Outside mark.
- **No model.** Without a link the conversation shows one line, "No model is connected", and the input line is disabled.

## 4. Flow between the screens

```
 Desktop/Launcher --Enter--> app starts (no access)
        ^                         |
        |                         | app asks for a file
        |                         v
        |               File-access request --Deny--> app gets "denied", stays running
        |                         |
        |                      Allow read
        |                         v
        |                 app shows the file --F12--> Workbench (Notes selected, grant listed)
        |                                                 |
        |                                             R, Enter
        |                                                 v
        |                              grant shows "Revoked"; the app's next read fails
        |                                                 |
        |                                  Esc, then Ctrl+Shift+X in Notes
        |                                                 v
        |                              Notes crashes; top bar shows "1 crash" in red
        |                                                 |
        |                                                F12
        |                                                 v
        +------Esc------ Workbench: Notes row "Crashed", its grant "Revoked" then "Released";
                         C opens the crash sheet
```

The Milestone 4 walkthrough uses exactly this path with the keyboard only: launch Notes, Allow, open the Workbench, Revoke, return to Notes and press its crash key (`Ctrl+Shift+X`; Notes then writes to address 0), open the Workbench and read Notes' crash sheet next to its revoked-then-released grant, return to the desktop. Faulter is a second, grant-less example: it crashes on its own and the other apps keep running. The walkthrough is run a second time with `ui.contrast=high`; those screenshots go to `build/m4-contrast/`.

## 5. Keyboard and focus rules (all screens)

- Every action has a key. The pointer is not required and is not implemented before Milestone 4 is accepted.
- `Tab` order is left to right, top to bottom, and wraps within a modal (focus trap); `Esc` always leaves the modal by the safest choice.
- Arrow keys move within a composite (table, list, shelf); `Tab` moves between composites and between single controls. In a modal sheet each button is its own Tab stop (APG modal dialog pattern); `Left/Right` also move between the buttons. `Enter` or `Space` activates the focused button. This is the "roving tabindex" convention of the WAI-ARIA Authoring Practices.
- Single-letter commands exist only in the Workbench and act only while its table or a grant has focus; in a text field every printable key is text.
- The focus mark is always visible, never clipped by another element, and never conveyed by colour alone (it is an outline that changes shape, not a tint).
- The bottom line of every screen lists the keys valid right now. The standard list movement keys (`Home/End`, `PageUp/PageDown`) are not repeated there.
- The **Still key** (default `F12`; decision D-UI-3) is reserved by the desktop. Apps never receive it. It always opens the Workbench, including over an open permission request, which then stays waiting, with its sheet covered, until you return to it (§3.3).
- From stage 4b `F10` (hands off) and `F8` (the assistant's access sheet) are reserved by the desktop as well (decision D-UI-8, section 3.4). Apps never receive them, and an assistant cannot send them.
- `desktop` also reserves `F11` (the launcher, from anywhere) and `F9` (cycle apps) (decision D-UI-6), and `Ctrl+F12`, which switches Contrast+. Apps never receive these either. `F9` and `F11` do nothing while a request sheet is open.

## 6. Building the look in stages

| Stage | Arrives with | Renderer | What produces the "glass" |
|---|---|---|---|
| 0 | Milestone 2 | Kernel text console on the framebuffer: solid fills, 8×16 bitmap glyphs (Spleen 8×16, shipped with its licence file from this milestone) | Nothing yet. Palette only: `void` background, `text`, `cyan` headings, `red`/`amber` status words. The text inspector already uses the state words (Running, Crashed, Exited); the glyphs and icons of §2.4 arrive with stage 1. |
| 1 | Milestone 4 baseline | User-space back buffer in RAM; rectangles, 1 px lines, per-row gradient fills, 1-bit glyphs; dirty rectangles copied to the framebuffer honouring stride and channel order | Panel = two-stop vertical gradient (`#131924` to `#0E131A`), 1 px top highlight (`#2C3A4C`), 1 px bottom shadow (`#04060A`). All translucent colours are **pre-flattened** at build time against the surface they sit on, so no per-pixel blending is needed. Dark gradients use a 4×4 ordered dither to avoid banding. |
| 2 | Milestone 4 polish | Add integer source-over blending (premultiplied alpha, done in linear light through a 256-entry `u16` table and a 4096-entry `u8` table), 8-bit glyph atlases, pre-baked rounded-corner and shadow sprites (9-slice) | Desktop wallpaper is blurred **once** at start-up (three box-blur passes) into a second buffer; panels sample that buffer, add a `panel` tint at 82% and a diagonal highlight band across their top third whose peak flattened colour on `panel` is `#1C222B` (about 1% white in linear light). Panels look translucent, but nothing is blurred per frame. The contrast rule below limits the wallpaper. |
| 3 | After the demo | Per-surface buffers with damage tracking; live backdrop blur at quarter resolution; SIMD blending once user-space vector state is saved per thread; page flipping when a native display driver exists | Real translucency under moving windows, 120–180 ms ease-out transitions, soft specular sweep on focus change. All of it disabled by `ui.motion=reduced`. |

Contrast rule for stage 2: the token table of §2.1 holds on flat surfaces, and a translucent panel takes part of its colour from the wallpaper. The flattened colour under any glyph must have relative luminance ≤ 0.0229, so that `text-2` stays ≥ 7:1 and `muted` ≥ 4.5:1. With an 82% `panel` tint blended in linear light, the blurred wallpaper must be no brighter than luminance 0.094 (sRGB grey 86/255) anywhere; under the peak of the highlight band the same rule gives 0.048 (grey 61/255). `xtask` composes the panel backgrounds from the shipped wallpaper (blur, tint, highlight band) and fails the build if any pixel breaks the rule. A wallpaper that has not passed this check is shown with opaque panels, as with `ui.transparency=reduced`.

Cost model for stage 1 at 1280×800×32 bpp: a full-screen copy is 4.1 MB; a typical Workbench update (one 742×28 px table row plus the 1280×32 px header) is about 250 KB, 6% of a full copy; a single changed cell is under 10 KB. With dirty rectangles the unaccelerated framebuffer is not a bottleneck for a keyboard-driven UI. Stage 2's one-time blur of the wallpaper is roughly six passes over 1 M pixels and runs once.

Memory: `desktop` holds one full-screen back buffer (3.9 MiB at 1280×800), from stage 2 a second one for the blurred wallpaper, and the surface of every windowed program, which it creates and which is charged to it (3.6 MiB per app and 3.9 MiB for the Workbench at 1280×800). At 1920×1080 the two full-screen buffers alone are 15.8 MiB. `desktop`'s entry in the boot filesystem table therefore gives it a limit of 64 MiB, or 8 × the framebuffer size if that is larger, instead of the 16 MiB that apps get (D-K-6; [03 §5.2](03-kernel-and-interfaces.md#52-paging-and-allocation)).

Correctness notes that apply from stage 1:

- The UI library draws in **RGBX8888 in the back buffer** and converts per format on copy-out (`BGRX` or `RGBX`, with stride = pixels per scan line × 4). Bitmask formats are rejected by the loader until supported (see [02](02-boot-and-handoff.md)).
- There is no vertical sync on a firmware framebuffer. Updates are small and atomic per rectangle, and nothing animates in stage 1, so tearing is not visible in practice.
- User programs are built without floating point or SSE until the kernel saves vector state (see [03](03-kernel-and-interfaces.md)), so all rendering math in stages 1 and 2 is integer.
- Blending in stage 2 converts each channel to linear light with a 256-entry `u16` table (sRGB 8-bit to linear 12-bit, 0 to 4095), blends, and converts back with a 4096-entry `u8` table (linear 12-bit to sRGB 8-bit). Blending the stored (gamma-encoded) values directly would make anti-aliased light text on near-black look thin and fringed, which is exactly this interface's common case. The two tables are 4,608 bytes of integers generated at build time, so no floating point is needed at run time. Pixels with alpha 0 or 255 bypass the tables, so opaque pixels stay bit-exact. 8-bit linear intermediates must not be used: they merge sRGB values 0–16 into two codes, which would band and tint every blended pixel of this near-black palette.

## 7. Reduced motion

`ui.motion` has two values. **`reduced` is what Milestones 2–4 ship**, because stages 0–2 contain no animation at all. When motion arrives in stage 3:

| Element | `full` | `reduced` |
|---|---|---|
| Open Workbench or modal | 140 ms fade and 8 px rise | Appears immediately |
| Selection move | 80 ms highlight slide | Jumps |
| Result strip | Slides in, auto-dismisses after 4 s | Appears; stays until the next key press |
| Crash badge | One 400 ms pulse | Static red badge with the word |
| Gloss highlight on focus change | Sweep | None |

Nothing flashes more than once, nothing loops, and no information is conveyed only by motion. The 500 ms input guard on the request sheet (§3.3) is not an animation and applies in both modes.

## 8. Inspector readability checklist

Used as acceptance criteria for the Workbench in Milestone 4:

- [ ] All text ≥ 4.5:1; primary and secondary text ≥ 7:1; focus mark ≥ 3:1 against adjacent colours.
- [ ] Every status shows glyph + word; a greyscale screenshot loses no information.
- [ ] Numeric columns right-aligned, monospaced, with units; no value changes more than once per second.
- [ ] Rows do not reorder unless the user changes the sort.
- [ ] The selected process, its activity and its access are visible together without scrolling at 1280×800 for up to 12 processes and 6 grants whose paths fit one line (about 40 characters); beyond that the grant list scrolls inside the pane and shows "n more".
- [ ] Every action reachable and reversible by keyboard; key hints on screen.
- [ ] Works with `ui.motion=reduced` and `ui.contrast=high`.

The fifth item rests on this arithmetic: at 1280×800 the Workbench has 680 px between its column header and its event strip (800 minus the 32 px header, the 28 px column header, event strip and key line, and four 1 px rules), which is room for 24 rows of 28 px. Twelve processes take 12 rows of the table. In the detail pane the activity block takes 6 rows, the access heading and the gap above it 2, six two-line grants 12 and the fixed endowments 2: 22 rows, which leaves one for a request waiting for an answer and one for the crash heading.
