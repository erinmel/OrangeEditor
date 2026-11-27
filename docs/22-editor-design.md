# Editor design direction

How the OrangeEditor canvas, panels and node families should look and behave. It is the
reference for editor work after 2026-11-16; earlier editor notes (02, 21) describe what
exists. It draws on node editors people already know (Blender's node editors, Unreal
Blueprints, Houdini, ComfyUI, Warudo) and on the owner's review of the current editor.
It changes no graph semantics; where a change needs one (11), it is called out.

## Workspace docks

Icon tabs live on the left and right rails. Top icons open side panels, and
icons at the foot of each rail open the corresponding half of the bottom tray.
Drag a tab to another area or onto another tab to place it before
that tab. The tab's context menu offers the same four destinations for moving
between areas. The side panels and bottom tray are resizable, and the two
bottom areas appear side by side when both are open. The editor remembers tab
placement, order, visibility, and panel sizes in application preferences.

Nodes, Variables, Types, and Connections start on the left, Properties starts on the
right, Graph Info starts at the bottom left, and Problems and Log start at the bottom
right. Right-click an icon to hide it, then right-click free space on a rail to add
it back in that position. Docks use padded surfaces and gutters to separate adjacent
panels. Resize edges become visible on hover or while dragging. Scrollbars expand
within a reserved gutter.

Settings → Display has pages for Appearance, Workspace, Toolbar, and Canvas.
Appearance configures corner radii independently for docks, windows, menus, and
controls. The toolbar options menu and its context menu let you show or hide
individual commands. In Customize toolbar, move commands earlier or later in the
left group or reset the toolbar. Check and Run are anchored near Settings and can
be hidden individually. Workspace controls icon names and status items. Optional CPU and network readings are
system-wide and sampled by the desktop. Manage packages and graph bundles in
Settings → Extensions.

## Finding nodes and connecting distant sections

In Nodes, Conversions expands into groups by source type. The catalogue generally
uses one or two group levels, with related nodes in expandable subgroups. Search
opens matching branches for keyboard navigation. Settings → Display can show or hide
branch guides and disclosure arrows and tint each branch and its contents. Related
nodes stay together in sections such as Wireless links and HTTP Request. Search
includes group and section names, node labels, and aliases; a heading explains its
purpose on hover.

Right-click the main canvas and choose **Add comment**, or search for **Comment**,
**note**, or **annotation** in the Nodes catalogue or canvas picker. Double-click a
comment to edit it, drag it to move it, or use its menu to delete it. Comments have
no ports and do not execute. They are saved with the graph and included when sharing
it as a bundle.

Guard searches find Branch. Rising Edge and Falling Edge each have a Boolean
input and a trigger output. When Value Changes Enough shows its tolerance on
the card and outputs the triggering sample. Increment Counter and Decrement
Counter live under Variables. Select the whole-number variable on the card,
leave Step at 1 or wire another step, and read the updated value from the output.
Loops includes paired loop placement and
standalone ends under Loop parts. Idle Watchdog is a starter graph. Use Make
Function for a named reusable call.

Value or Fallback (Coalesce) takes an optional value and uses its If missing input
only when the option is absent. For example, connect an optional display name and
give Guest as the fallback. A present empty name is still a value. Release a wire
from an optional output over empty canvas to find Value or Fallback first.

For an enum, connect it to Switch's key or choose its type, then select Add enum
cases under Ports. Existing variant branches keep their connections. New cases
use the enum's display names and persistent variant IDs. Releasing a wire from an
enum output also offers a Switch preconfigured with those cases and, for variants
that carry data, a payload reader. Releasing a wire from a record output offers a
reader or a copy with a changed value for each named field. Placement and wiring
are one undo step.

Dispatch (`core/emit`, ES "Despachar") sends an event on a named channel, a run
event (11 §10), to every Listen (`core/on-event`, ES "Escuchar") on it: one to many.
Each Listen starts its own activation and shows the payload output only when the
channel has a payload type. A Dispatch and a Listen share the channel's name.
The node palette offers one Dispatch and one Listen. On a Dispatch, + Add Subscriber creates a Listen on the same channel that
follows the pointer until a click places it; Escape leaves it where it is. On a fresh
Dispatch this action also creates a pulse channel. Use the card's channel picker to
select an existing channel, create a new one on Dispatch, or rename the selected
channel. Rename changes the shared name without rebinding any nodes. A Listen
placed on its own picks its channel from a searchable picker on the card, or comes bound from the node
search: searching for Listen, or for a channel's name, lists each channel a Dispatch
sends on as "Listen · channel", placed bound in one step.

Deleting a Dispatch never deletes its subscribers. A Listen no Dispatch sends to shows
an "Unbound · channel" tag above its card; clicking it creates a Dispatch on that
channel (following the pointer), and choosing another channel rebinds it. Creating
an emitter from a Listen with no channel creates a pulse channel in the same edit.
Edit the shared channel name in Properties. A Dispatch with no listeners still continues on its normal exit. Pass shared
information as the event payload or through a declared variable.

Bridge Out (`core/bridge-out`, ES "Puente salida") shares a value without a wire,
and each Bridge In (`core/bridge-in`, ES "Puente entrada") on its channel reads it
when pulled: one to many, with no execution flow, and unlike a variable nothing is
stored or written. A new Bridge Out gets an unused channel name ("Bridge 1",
"Bridge 2", …) that can be renamed in Properties. On a Bridge Out, + Add Subscriber
creates a Bridge In bound to it that follows the pointer. A Bridge In placed on its
own chooses its Bridge Out from a searchable channel picker on its card. Renaming a
Bridge Out updates each bound Bridge In's saved channel name in the same undo step. The binding is the data edge
from Bridge Out `result` to Bridge In `value`, drawn as a wireless link rather than
a wire, so those two ports are not shown. Deleting a Bridge Out keeps its Bridge Ins
with the channel name they had and an "Unbound · channel" tag; clicking the tag
creates a Bridge Out on that channel, and the picker rebinds it.

Wireless links run from each Dispatch to each Listen on its channel, and from each
Bridge Out to its Bridge Ins, with the same
spline as ordinary wires, dashed. Settings > Canvas > Wireless link opacity (0–100 %,
default 10 %) sets how visible they are at rest on a perceptual curve: 0 % hides
them, 1 % is faint but visible, 100 % is opaque. Above 0 %, hovering a wireless
node shows its links fully, and resting the pointer on a faint link for one second
shows every link; leaving earlier cancels it. Links fade in and out. At 0 % they stay
hidden on hover. Event links are saved only through the event references on the
nodes, and bridge links as their data edges.

For a typed payload, place Dispatch, Listen or Await Event, then open its Event
picker in Properties. Choose New event, select a payload type, and create it.
Creation assigns the event to that node in one undoable edit. Select the same
named event on the other nodes. Listen starts a new activation, while Await
Event continues an existing one when the event arrives or its timeout expires.

Choose No payload (pulse) to create an event-only channel. Dispatch and Listen
then have no payload ports. Idle Watchdog places Dispatch, Listen, and Wait from
left to right on an event-only heartbeat channel. Newly placed presets are selected together so they can move as a set.

Collect starts with a limit of 1,024 items. Tee starts with buffers of 32 items
and 1 MiB per output, with backpressure when either bound is reached. Configure
the limits in Properties for the input stream. With backpressure, drain both
branches concurrently or close the branch that will not be used.

For a larger automation, group the trigger, data preparation, and effects where
each can be inspected independently. For example, a timer can start an HTTP JSON
request, route its success through JSON Field and a typed conversion, and use
For Each for a side effect per item. Use Repeat with State to carry a result from
one iteration to the next, or Make Function to expose typed inputs and outputs for
reusable calculations. One Wireless Signal can reach independent destinations
such as a log and a dashboard. The CLI starter graphs in the README show the
corresponding HTTP, loops, failure and event patterns.

In Settings, the theme tiles preview and select System, Light and Dark. System
shows both palettes on a diagonal and may preview additional sidebar, canvas,
button and text surfaces. Editing a shortcut immediately waits for a
key combination, showing held modifiers as you press them. Confirm it to save,
use Escape to cancel, or explicitly reassign
a conflicting binding. Search accepts the displayed action, category and key
combination in the selected language.

## 1. Principles

1. **The canvas is where work happens.** Anything a person sets often (a number, a
   choice, a device, a file) is set on the card. The properties panel is for the long
   tail, never the only way.
2. **A graph explains itself while it runs.** Every port shows the value it last carried;
   a node says what it computes (`a > b`); a trigger shows what it fires with. Debugging
   needs no log reading.
3. **Problems are quiet until asked.** A problem marks the exact place (a port, a wire, a
   field) with a small sign; full diagnostic text lives in the Problems dock. Nothing is
   drawn over other cards.
4. **Words a person uses.** Names say what something is for (Int, Text, Microphone in),
   search understands intent ("trigger", "when", "every second", "disparador"), and
   symbols are icons with tooltips, never missing glyphs.
5. **Direct manipulation.** Drag a node onto a wire to insert it, click a wire to select
   it, Alt-drag a wire to cut, right-click for what can be done here.

## 2. Canvas

| Area | Decision |
|---|---|
| Wheel | Zooms about the pointer by default. Shift+wheel scrolls sideways, Ctrl+wheel up and down. A setting gives back scroll-first for trackpads. |
| Grid | A magnet icon toggles Snap to grid. Grid settings uses one segmented radio group for Dots, Lines and Hidden, with one highlighted choice and arrow-key navigation. The spacing slider shows four stops at 12, 24, 36 and 48 graph units. The same controls are available in Display and canvas. Grid visibility is independent of snapping. The default spacing is 24 graph units. Round dots and pixel-aligned lines use stronger marks every four steps. Below 12 points between minor marks, only the stronger grid is drawn. Both grids keep their alignment as the canvas pans and zooms. Alt while dragging moves freely. The zoom ranges from 20% to 175%. The zoom percentage opens a continuous slider, Fit and Actual size. The slider keeps the center point of the viewport fixed. |
| Resize | The desktop uses the platform window frame for resizing. Resizing the window or a dock preserves the graph point at the center of the canvas. |
| Overlap | Selection, dragging, and completed inline edits bring a card forward. The last active card stays in front when selection clears. A front card covers only the area it overlaps and receives pointer input there. Exposed controls on rear cards stay visible and usable. |
| Wires | Execution ports are diamonds, stream ports are squares, and other data ports are circles. Execution wires are thicker. Connector hues are adjusted for contrast against the canvas and card surfaces. Hovering a port shows its full label and type. A wire can be hovered (it thickens), clicked to select (Delete removes it), and right-clicked to insert a node, set or remove a reroute point, or delete the wire. Reroute points can be dragged and are saved as layout, independent of execution. Dragging a node over a wire highlights it; letting go inserts the node between both ends when its ports fit (the model's insertion transaction, 12). |
| Values | Data ports show compact previews inside the card, with room reserved for the port labels. Clicking a preview or its label opens a scrollable, read-only inspector. Copy includes the complete observed text or JSON even when the preview is shortened. The editor previews pure outputs whose authoring inputs are known and shows completed effect and variable-update outputs even without outgoing wires. Previews read JSON as its text, enum values by choice name, collections as `[1, 2, …]` and `{key: value}` summaries, and records by type name; long decimals in number fields show five places. |
| Problems | A small red or amber indicator at the top-right of the header reveals details on hover. Faulty ports and wires use the same diagnostic colour. A port outline shares the connector centre and follows its diamond, square or circle at every zoom. Clicking a validation marker opens Problems and clicking a runtime failure opens Log. The two views have independent search, filters and scroll positions. |
| Run feedback | The reported execution route glows along the whole wire and fades in 160 ms. A stream wire with a current runtime sample shows small moving dots. A failed step retains its diagnostic dot until it succeeds. |
| Empty | A graph with no nodes shows, at the centre of the canvas, how to begin: Tab or right-click to add a node, or drag one from the Nodes panel. |
| Groups | Frames are drawn under cards and move by their title bars, including when collapsed. Snapping moves every member by the same offset and anchors the frame at its upper-left corner. Clicking a collapsed frame selects it for Ungroup. |

Click either connector, then click the opposite endpoint to wire without dragging.\nPress Escape, click the starting port again, or click the background to cancel.\n\n## 3. Cards

* **Titles.** The header uses a category symbol such as a calculator for math or a
  speaker for audio. The node name appears white in dark mode. Double-click the
  name in Properties or on the card, or choose **Rename node** in the card menu.
  Rename replaces the title where it was double-clicked, including on nodes supplied
  by a package. The card menu opens the field in Properties. Enter applies the name.
  Escape or leaving the field discards it. Keep the title baseline in the same
  position in display and edit mode.
  At overview zoom, titles keep a readable minimum size and truncate to the card.
  Clearing it restores the type label. The tooltip on a renamed card still identifies
  its node type. The edit participates in Undo.
* **Functions.** Shift-click cards to select them, then choose **Make function…**
  from the card menu or the selection inspector. The preview shows the inputs and
  outputs formed by wires that cross the selection. Leave argument sources and
  result consumers outside the selection to expose those ports. Confirming puts a
  call node in their place and stores the selected body in the graph. A duplicated
  call uses the same function with its own wires and inputs. A selection containing
  Count, For Range, While or For each makes an async function. Triggers stay on the main
  graph and can be wired to the call.
  **Open function** on a local call opens the shared body in its own canvas.
  The interface cards show how arguments and results cross the boundary. Edits to
  the body use the graph's undo history and apply to every call. The saved graph
  holds the typed interface and body, which the compiler lowers for preview or run.
  To share a function between graphs, **Export function…** on its call shows its
  public ports and exact provider package versions before saving an archive.
  An imported card keeps its definition, label and connection picker.
  **Expand function** replaces one local call with editable copies of its body and
  reconnects its inputs and result in one undo step. Other calls still use the saved
  definition. Expansion currently supports non-generic pure functions with one result
  from a body card. Wired arguments require a single strict body card that reads them
  in interface order. Loop regions require their own expansion support.

* **Inline values.** A literal input is edited on the card: a single click on a number
  types it, a drag scrubs it; text is typed in place; a choice is a dropdown; a path
  opens the system's file or folder chooser on click.
* **Expressions.** Operators show what they compute in a dedicated body row with their inputs'
  names or values: Greater shows `a > b`, or `12 > 7` when both are known. Comparison,
  arithmetic and logic nodes get this from their definition's `formula` presentation hint.
  All Conditions and Any Conditions start with two boolean inputs. Add a condition
  on the card to check more values in the displayed order.
* **Loops.** For Range exposes Start, End and Step as wireable inputs with defaults
  of zero for Start and one for Step. While exposes Condition on the card.
  Body runs each iteration and Done runs after completion.
  For Each displays its default List items mode on the card and in Properties.
  Choose Map entries to work with key and value ports. Wiring a list or map
  sets the matching mode. Switching modes clears the old collection
  literal and the wires for ports that disappear. Choosing a mode manually also
  disconnects the old collection input. Undo restores the mode, values, and wires
  together.
  The iteration limit is in Properties under Advanced.
  Placing Repeat with State, Map, Filter or Reduce from the Loops group creates
  both loop boundaries and a passthrough wire in one undoable edit.
  Replace that wire with the body you want to run for each item or step.
  Duplicating the pair creates an independent loop with the copied wiring.
* **Triggers.** A trigger shows its contents: On manual has a Fire button in the body
  (disabled, with the reason on hover, while the graph is not running) and its payload
  field; On timer shows its interval; an event trigger shows the event it waits for.
  Rising Edge and Falling Edge fire on false-to-true and true-to-false
  transitions, respectively. The initial input establishes their state.
  Guard places a Branch with a boolean condition. Wire its True output to
  continue when the condition passes.
  A new On timer fires every second, first after one interval, and a new On timeout
  after one second, so a placed timer runs without further set-up.
* **Variable writes.** Set and Update cards show what they do under the flow row,
  with the typed value or the wired input's name: `Counter ← Counter + 1`,
  `Counter ← 0`. A renamed card ("Add one") keeps showing its operation. Properties
  repeats the line.
* **Value nodes** (Int, Float, Bool, Text) are compact: one row, the value itself.
* **Arithmetic.** Add and other arithmetic cards use their named ports and values.
  Comparison cards may retain a comparison expression. Ordinary arithmetic has
  no redundant expression footer.
* **Configuration templates.** Scalar controls are drawn from each definition's
  schema. Network connection references are dropdowns of compatible saved
  profiles and display the selected name. Profile editing lives in Connections.
  The default picker order is HTTP/HTTPS, WebSocket, AMQP, MQTT, Folder, then an Audio
  group with the source and output adapters, followed by other adapters. The desktop Folder form opens the
  native picker for its root. File nodes using that profile store paths relative
  to the chosen root. The path field names the folder connection, and desktop picker
  errors name the connection when it is known. Embeddings without a dialog handler
  show a text field.
  Device nodes have an inline device selector. Fields preserve their JSON types.
* **Properties** shows the node's name, kind, description and set-up, its inputs, an
  Outputs list with each output's type, observed values, and a destructive Delete
  node. A record or enum node names a graph's own type by its declared name.
* **Language.** Canvas titles use the same localized catalogue labels as search.
  User-assigned node names are preserved. Port labels use the host's supplied text.
* **Composition.** Embedding applications may call `Editor::ui_with_layout` and
  place or omit the toolbar, status bar, catalogue, inspector, diagnostics and
  editable canvas in their own containers. Toolbar and context menu command
  glyphs use the same centered renderer and support a host painter. The live
  shortcut is shown when assigned. Dropdowns use the local surface fill, a
  centered chevron and the theme's selection outline on focus or when open.

  The private `orange-ui-controls` crate owns button roles, dropdowns, search
  layout, icon button geometry, disclosure rows, notebook tabs, segmented radios,
  continuous and stepped sliders, marked sliders and circular knobs, checkboxes,
  switches, simple and emphasized radios, text fields and text areas, shared
  dialog and popup frames, and reusable level-meter and chart representations.
  Notebooks support horizontal and vertical orientations with compact or
  comfortable padding. Editor dock tabs fill the available width and move to an
  overflow menu when the labels no longer fit. The tab row and search field use
  the visible panel width during resizing. Buttons expose solid, outline, ghost and subtle visual
  styles plus persistent toggle state, hover, pressed, focused and disabled states.
  The editor owns card layout, port geometry, wiring, document settings and commit rules.
  Both search and placement previews use the same card layout as the live canvas.
  The normal template reads the host projection of typed ports, defaults, references,
  and settings. A specialized renderer checks the projected shape it consumes and
  falls back to the normal template for incompatible definitions. Presentation
  does not change which ports the graph can wire.
  Geometry presets cover rounded, compact, pill, no-border and square buttons.
  Semantic tones cover neutral, info, success, warning and danger. Confirmation
  pairs keep Cancel neutral and Confirm on a selected semantic tone. Border,
  radius, fill and foreground are host-theme tokens shared with every other control.
  Meter and chart modules own continuous, segmented and line/area/bars drawing;
  OrangeEditor supplies normalized audio values and graph-specific thresholds.
  The semantic
  palette exposes text, muted text, panel, dialog and inset layers, plus colors
  and foregrounds for primary, secondary and destructive actions. Apps can
  override those tokens per egui context. Tertiary actions and quiet menu
  buttons gain emphasis on hover. Hover and press transitions follow egui's
  animation time, including immediate state changes when it is zero.
  Apps supply their own icons and labels. Connection form actions wrap as the
  panel narrows. Notebook tabs never change height: when a narrow dock cannot
  show every tab, overflow tabs move into a fixed-height dropdown while the
  selected tab remains visible.
  Panel notebooks fill their available panel width: left and right docks distribute
  compact tabs across the row, and the bottom Problems/Log notebook aligns its
  search and content beneath the same header. Fixed-width notebooks remain opt-in
  for dialogs and toolbars.
  Notebook padding is compact by default: text has a short inset, the frame
  has two to four units of inset, and tab height remains 28 units. Comfortable
  padding is an opt-in alternative for larger dialogs, never a narrow-dock default.
  Two-option and multi-option selectors use an intrinsic pill track with an
  animated active pill. Options stack when their measured labels and padding
  exceed the panel width. A single pill carries the selection through the
  entire transition. A small underline identifies keyboard focus.
  Every button family uses the same interaction precedence: disabled suppresses
  interaction and lowers contrast; selected or active is persistent; pressed is
  transient while held; focused has a visible outline independent of pointer
  hover; hovered lightens its fill; inactive uses the rest state. A boolean
  toggle exposes on/off; a segmented radio or radio list exposes one choice;
  a checklist or multi-select exposes several independent choices.

  The shared component theme supports System, Light, Dark and custom palettes,
  plus High Contrast Light and High Contrast Dark. The high-contrast palettes
  use text and semantic inks at 7:1 or higher against their surfaces, prominent
  borders and two-point focus rings. Notes and validation states pair color with
  a symbol or text. Hosts supply localized accessibility labels, and custom
  actionable shapes expose egui widget roles and state. The component defaults
  are Compact at 24 points and Comfy at 40 points, with spacing multiples
  `[1, 2, 4, 6, 8]` of two and three points respectively. Editor desktop
  defaults use Compact. The shared modal opts into Comfy.

  Run `cargo run -p orange-ui-controls --locked --example gallery` to see all
  component examples on one scrollable canvas. Appearance tiles follow the
  Settings layout, including the split System preview, and apply the palette
  to the whole gallery. Compact and Comfy apply the density across the page.
  Button roles, continuous and stepped sliders, disabled controls and
  validation examples appear inline. Gallery rows measure their tallest card
  from child content. Menu lists use full-width borderless rows, a dedicated
  icon column and muted shortcut hints. Structured dialogs use a fullscreen
  dim backdrop, separate header, scrollable body and footer, and Escape close.
  The WAV preview decodes on a worker and optionally plays a bounded mono
  preview on the default playback device after explicit activation.
  Section links live in the header so the canvas uses the full window width.
  File fixtures browse or accept file drops and list selected names. Clock
  wheels support focused arrow keys, scrolling and vertical pointer drags. They wrap hours at 24 and minutes and seconds at 60, independently.
  Search has a pill-shaped inset field and leading icon. Plain inputs use a
  rectangular inset, dropdowns add a chevron, and text areas offer resize grips.
  Input borders and active toggle text meet the gallery contrast checks.
  The gallery's audio preview uses four adjustable biquad bands, a logarithmic
  20 Hz–20 kHz plot, a 2048-sample Hann FFT of a 48 kHz synthetic trumpet and a
  post-EQ waveform. Its stereo dBFS meters use separate peak, RMS and held
  values, 6 dB ticks, and per-channel clipping latches. The demonstration runs
  on the UI side and uses a pausable signal. Visible signal displays target 60 Hz and the spectrum targets 30 Hz. Pausing or scrolling all live displays out of view suspends signal processing. EQ response samples are cached by band settings, sample rate and plot resolution.
  Continuous and stepped sliders share a thin rail and a thumb. `SliderStyle` sets
  how they look, for the whole application (`install`) or a group of sliders (`scope`):
  notches are pills on the rail (the house style), ticks under it, or dots, and the
  thumb is a capsule (the house style), a circle or an upright bar. Notches light once
  the value has passed them. Pill stops closer than a pill's width become ticks, so a
  dense range reads as a ruler; ticks are taller where a label sits. A logarithmic
  slider marks 1, 2 and 5 of each decade, the powers of ten as major stops. Labels sit
  below the rail and its notches. A stepped slider changes by one step with the arrow
  keys.
  Rotary knobs offer arc, center-detent bipolar and marked stepped styles, plus a repeating encoder whose upper bound wraps to its lower bound. A dial (`orange_ui_controls::dial`) is the self-contained form: a ring of fine ticks lit up to the value, the value and unit inside, and its name in the ring's open bottom quarter, so it needs no label row.
  Drag up or right to increase the value; a full sweep takes 200 points, Shift is ten
  times finer and Ctrl (Command on macOS) a hundred times. Focus the knob to nudge it one
  step with the wheel or arrow keys, ten with Page Up and Page Down, or double-click the
  centered readout to enter a number.
  Continuous marked sliders show guide notches without snapping. Canvas zoom
  uses guides at 50%, 75%, 100%, 125%, 150% and 175%, but accepts every value within 20–175%.
  A guided slider may hide guides, show only notches, show only labels, or show
  notches and labels. A strict guided slider snaps to its configured step; a free
  guided slider never snaps. Circular controls share the same guide and step
  concepts for values that suit a dial.

### 3.1 Card anatomy (rules, 2026-11-18, revised)

Every card follows these rules. A card that needs to break one is a design change
here first. One row model (`canvas/card.rs`) builds a card's rows once from its view;
size, port positions, wires, hit tests, painting and the controls all read it, so no
other code places anything on a card.

* **A card is a stack of rows, full width.** Standard cards are 272 units wide.
  Literals are 180 units wide and audio chart cards have dedicated widths.
  A field spans the card's inner width, so values line up
  from card to card and nothing is squeezed beside a name.
* **Flow first.** The first row holds the way in on the left (the tinted Enter chip, or
  Fire on On manual) and the first way out on the right (Exit →). Further ways out
  take one row each, right-aligned. An input and an output may share a row; that is
  the only side-by-side placement.
* **Outputs are one row each**, the name right-aligned beside its dot, and while a run
  reports values, the latest value in a pill before the name. A stream output shows
  no value. An input that has no field (wired, or a type nobody types, such as a
  stream) shares the first free output row on its left.
* **Every editable value is a field block**: its name on one line, the full-width
  control on the next. An input connector sits halfway across the height of the
  name and control, excluding the trailing gap. Resizing a multiline field
  recenters the connector, wire anchor, hit target and diagnostic outline together.
* **One control per kind, always the same look:** true or false is a No | Yes switch;
  a choice (enum, device, variable, key) is a dropdown; a number is a field dragged to
  step and clicked to type, with its unit; text is a field clicked to type, elided with
  "…" and shown whole on hover; a file or folder is a field with the name and a browse
  button inside its right end. An empty field shows the default dimmed, or "—" when
  there is none.
* **Ports a node may have more of grow on the card.** A variadic port (a Fork's
  branches, a Sequence's steps) ends the card with a quiet "+ Add branch" button until
  it reaches its most; removing one is in Properties.
* **A wired input keeps its name line only**, with the value it received in a pill
  at the right while a run reports it. Unwiring brings its field back.
* **Typed input details** are shared between the card and Properties. A port's
  help distinguishes an inherited default, an implicit None, a required value,
  a saved literal, and a wire shadowing a saved literal. Option and aggregate
  controls edit canonical values and apply one validated edit. Make Record can
  explicitly omit schema-optional fields. Build Map offers paired rows while
  both typed input lists are unwired.
* **Loop cards** identify begin, end and result. Before an authoring gesture
  changes a For Each between list and map, the editor shows the affected Items
  value or wire, type pins and body wires. A confirmed change is one undoable edit.
* **Fire With Value** opens a transient typed sample for a running root manual
  entry that accepts a payload. The host validates it against that running
  incarnation and rejects submissions after the flow is replaced.
* **Saved ports** remain available for inspection and existing wires when their
  provider is unavailable. A wire whose saved endpoint has no matching port
  has a repair marker. Backward wiring highlights matching live outputs using
  matches computed for the active drag. A converter chosen for a splice is one
  edit, with the original wire and waypoint restored on undo.
* **Failure routes** are available below the primary rows on cards with implicit
  failure or cancellation ports. Expand the section with its button, with Enter or
  Space while focused, from Properties, or with the selected card shortcut
  (Alt+O by default). Wired routes, a pending wire's source and the focused
  diagnostic port stay visible when collapsed. A wired Error also keeps Failed
  visible. Error is available only on the Failed path. The disclosure is view state
  and leaves primary port positions and graph edits intact.
* **Edit card values in place.** Controls such as the color picker can open an
  anchored popup. Tuning settings such as the analysis window live in Properties
  under Advanced.
* **No redundant words.** No type labels (hover shows the type); a field's name is
  left out when it only repeats the card's title (Audio source's device list, Get
  variable's variable).
* **Problems stay with the card.** A node's problem is an icon at
  the right end of its header, in space the title leaves free. A port's problem is a
  thin ring centered on its dot. Both are painted with the owning card. Hovering the
  icon says what is wrong. Clicking it opens Problems.
* **Defaults read like values.** An empty field shows its default the way a value is
  shown (an enum by its choice's name), dimmed.
* **Measurements:** header 30, flow and output rows 28, a field block 52 (name 18,
  control 26, gap 8), padding 10, the card ends 8 units after its last row. Text never
  overflows its row: names are elided and shown whole on hover.
  The Properties panel wraps result text at its current width, including long
  unbroken strings, and scrolls within the result. Copy keeps the full value.

#### Where the rules live in code

* `canvas/card.rs` is the only place a card's geometry is decided: its rows, their
  heights, where each dot is, where each control goes, and every metric above.
* `canvas/fields.rs` is the only place a value's control is drawn and read; the
  card and Properties use the same kinds (switch, dropdown, number, text, path).
* Colors come from the host application's egui `Visuals` (`Palette::of`), so an
  application that embeds the canvas restyles cards with its own theme. Metrics are
  constants in `card.rs`; making them a runtime theme is a change to that one file.
* A new kind of value gets a `Kind` in `fields.rs`; a new kind of row (for example a
  level meter or a spectrum drawn in the card) gets a `Left`/row variant in
  `card.rs` with a fixed height, so wires and hit tests keep working. A package never
  draws on a card directly.

#### Review gallery

`orange-desktop` builds and reopens gallery graphs with every searchable node.
Set `ORANGE_GALLERY_DIR` and run `cargo test -p orange-desktop package_galleries`
to save a copy. Open each with `--canvas-only --screenshot` and
check every card against these rules after a rendering change.
* **Audio charts.** Frequency Splitter reserves a collapsible plot above its port
  rows. Audio Level Trigger places its meter beside its outputs and settings.

## 4. Search and commands

Palette and canvas search use the same full-width name rows. Purpose and category
appear together on hover. Up and Down move the highlight, and Enter adds the
highlighted result. Dragging a result shows a placement preview under the pointer.
The palette uses the shared search field with a clear action and compact category
rows with disclosure chevrons. Node search settings use a segmented control with
Names only on the left and Node cards on the right.

* The Nodes tab groups related nodes by task, including Control flow, Triggers &
  signals, Variables, Text, JSON, Collections, Files and Network. The Conversions
  tab groups explicit converters by input type and lists each selectable policy.
  In the browse tree, Dispatch precedes Listen and Bridge Out precedes Bridge In.
  Canvas quick add still includes converters, with compatible choices on value
  wires. Converter previews use the selected contract's input and output ports,
  including a loss output where declared. The placed card uses the same conversion
  title and policy labels. Conversion names read as source type to destination
  type, with encoding or policy details where needed. JSON value extraction
  includes Optional in the destination name because it can return no value.

* One search for the palette, the wire-drop list and the quick-add box. It matches
  names, aliases, categories, concepts, descriptions and common configuration captions in the
  active language and in the bundled search languages. Concepts group nodes by purpose: *trigger / when /
  start* finds every entry kind (On manual, On timer, On start, On event, On level);
  *math*, *compare*, *text*, *time*, *audio*, *network*, *file*, *variable*, *loop*.
* Direct names and aliases in the active language rank above direct matches in
  fallback languages. Within each language tier, exact phrases precede prefixes
  and word matches. Concepts, descriptions and approximate matches follow.
  Spaces, hyphens and underscores between
  words are interchangeable. Search accepts one insertion, deletion, substitution
  or adjacent swap per word of four or more letters. Settings, shortcuts, channels,
  declarations and package pickers use the same matching rules. Each node result
  has a category and purpose on hover. Arrow keys move through results and Enter adds.
* Concepts and aliases are translatable catalogue entries (13): a package may add its
  own, and a locale adds words without changing IDs.
* **What a wire offers** (2026-11-24). A wire let go on empty canvas lists the kinds
  that take it (`Host::takers`); Insert node… on a wire lists the kinds that take its
  output and go on into its input (`Host::splice_takers`). Matching binds each kind's
  type parameters once and checks their constraints (`orange_model::fit::PortFit`), so
  a whole number never offers List Length, a `list<i64>` wire into a `list<i64>` input
  offers List Reverse but not List Length, and a decimal offers Greater but not Equal
  (11 §5.3). A step goes on through `then`, else a loop's `completed`; failure,
  cancellation and loop-body ports and triggers are never offered as guessed
  continuations. Explicit conversions stay listed for a value wire let go on canvas.
* **Wires from inputs.** A wire can also be dragged back from an input. Let go on an
  output of the same sort, it connects there. Let go on empty canvas, the search lists
  the kinds with an output that feeds the input (`Host::givers`, same matching), with
  triggers for a step input and converters that produce the value input's type. The
  selected node's output is wired into the input.
* **One action, one undo.** Choosing from a dropped wire, from either end, or a
  conversion for it, places the node and wires it as one edit (`Host::grouped`), like
  Insert node…: one undo takes both back. A choice
  the language refuses, or a graph that changed while the search was open, changes
  nothing: the graph, the selection and the undo and redo history stay as they were,
  and the Log says why.
* The right-click menu is the command centre: on the canvas (Add node… with search,
  Paste, Group, Arrange, Snap), on a card (Properties, Rename, Duplicate, Disable,
  Group, Make function, Delete, Help), on a wire (Insert node…, Delete), on a port
  (Disconnect, Promote to variable, Watch value). Entries show their assigned shortcuts.
* The toolbar uses icons with tooltips grouped as File | Edit | Run. Most
  icons use the pinned Material Symbols font. Group, Ungroup and Arrange use
  outlines adapted from Lucide, and Snap uses the Tabler magnet. Source versions
  and licenses are recorded in 10.
  Group, Ungroup, Arrange, Unlink and Fit to view use the same icon buttons as
  Undo and Redo and sit next to them in the same row. Selection actions are enabled
  when the selection supports them. Snap to grid shows its active state on the toolbar.
* The workspace starts with both side docks visible. The bottom bar puts the node dock toggle
  and zoom at the left, the inspector dock toggle at the right, and Problems, Log and
  running status in the center. Properties and Packages share the inspector's top tabs.
  Problems and Log share the same resizable panel. The controls wrap in narrow windows.
  Ctrl/Cmd+1 and Ctrl/Cmd+2 toggle the left and right docks. The right dock
  reopens to its last tab. Ctrl/Cmd+G groups the selection and Ctrl/Cmd+Shift+G
  ungroups the selected frame. Alt+G switches snapping.
* The desktop title follows the current file name. Running, Stopped or No graph
  and the unsaved indicator remain in the status bar.
* Settings uses a centered, resizable dialog with a search field above the text
  navigation in the left sidebar, an independently scrolling content pane, and a
  Done button spanning the left sidebar. On compact windows with a top navigation
  strip, Done uses the bottom footer. General groups Language
  and Node search. Node search displays either names or read-only canvas cards with
  their ports and charts directly in the results. The same choice applies to the node
  tree and canvas quick-add. Search and the section selector share a single strip in
  compact windows. Search finds settings sections, shortcut
  actions and key combinations. Display and canvas groups controls by interface,
  including the persisted System, Light, and Dark appearance choices,
  panels and canvas. Choose the Light or Dark preview to apply that theme. The
  System preview shows both palettes on a diagonal and follows the OS. Choice labels align
  in a column when space permits and stack above their controls in compact windows.
  Interface scale applies when a percentage is chosen in its dropdown. At large
  scales or narrow widths, the editing commands can scroll
  horizontally so Settings and the run controls stay visible.
  The Keyboard shortcuts page displays grouped, left-aligned action labels and
  single-line bindings. Select a binding to record a chord in its cell while
  keeping the same row height. Confirm or edit the captured chord there. Shortcuts
  can be removed or reset, and conflicts require
  explicit reassignment. The desktop saves bindings, node language and canvas preferences
  under the user's configuration directory on exit. Graph documents carry none of
  these preferences.
  The desktop applies the draft's blue-gray surface hierarchy. Command icons use
  a consistent 16-point outlined font. File and edit controls stay at the left,
  execution and settings at the right. One execution control shows a green Run
  arrow when available, gray when blocked, and a white Stop icon on red while
  running. Icons are centered within their buttons. Hovering an enabled button
  highlights its background, including the red Stop control. Menu and toolbar
  shortcut labels follow the current binding.
* Problems has a search field and an inline severity dropdown for Blockers,
  Warnings and Drafts. The dropdown and search are on one flex row; no extra
  filter row is drawn.
  Each row has a checkbox for multi-selection. Ctrl/Cmd+C copies only selected
  rows as formatted plain text with severity, diagnostic code, affected node
  and message. **Clear selection** clears the checkboxes and **Copy selected
  problems** repeats the same plain-text copy.
  Log uses the same element-first flex-row rule: level filter, live-trace and
  clear actions come before search. Problems and Log do not add a separate
  notebook inside the bottom diagnostics panel.
  The **Show the node** button jumps to the affected node.
  Search matches node names, text and diagnostic codes. Blockers appear first.
  The workspace badge counts blockers and warnings. Drafts have their own count in
  the panel.

| Problem | Effect on the run |
| --- | --- |
| Blocker · red error circle | An error affects a connected or exported value, an execution path, a wired trigger, or the structure of the runnable graph. Fix it to run the draft. A rejected live edit leaves the last working snapshot active. |
| Warning · amber triangle | The validator found an advisory condition, such as a shadowed input or a stale write. It permits execution. |
| Draft · muted pencil | A node still needs an input, reference, type, configuration, zone partner or execution path. An isolated card can stay out of the run. An unused action may already have data inputs if it has no execution wire or consumers. |

Unfinished standalone triggers can stay in Drafts even when they are the only
cards. An Audio Level Trigger or Frequency Splitter without an input stays idle
while its outputs are unused and the node is unwired. Wires feeding triggers or
passive nodes, outgoing wires, and exports count as uses. The editor retains draft
cards and their wiring.
When an omission removes a wire or an unfinished zone marker, the editor checks the resulting snapshot for errors before
enabling Run. A ready or connected draft card can join the next validated live
update. These categories are editor presentation and run preparation. Graph
validation keeps the diagnostic codes and severities defined in 11.

### 4.1 Log

The Log shows run lifecycle, runtime faults and messages written by the graph.
Search matches message text and level names. The level dropdown filters CRITICAL,
ERROR, WARN, INFO, DEBUG and TRACE. The Live trace toggle shows its state and
records successful steps and live values at TRACE while on. Clear log removes the
history. Consecutive graph messages with the same text and level share a line and
a count. At most 256 graph messages wait for the UI, and excess messages are counted.

The **Log message** card shows execution ports, Log level, Message and parts in
that order. Log level defaults to INFO. Add part creates up to 24 text input slots
on the card. Each slot can hold typed text or take a wire. The remove control on
each row removes that part and its wire. Values of other types use explicit text
conversions where available.

New nodes use definition version 3. The message prints first, followed by each
nonempty part on a separate line. Sequential `{}` and zero-based `{0}` placeholders
place parts inline. A part used by a placeholder is skipped when appending the
remaining parts. Doubled braces write literal braces. Unknown placeholders stay
visible. A message is bounded at 16 KiB at a valid UTF-8 boundary. Saved version 2
nodes keep placeholder-only formatting. Saved version 1 nodes keep their original
severity choices and literal messages.

### 4.2 Shared controls

Use `orange-ui-controls` for reusable layout and presentation. The editor's
public `orange_editor_ui::controls` module supplies its localized labels and
icon defaults. Applications embedding the editor can reuse the controls with
their own egui theme.
Choose `Surface::Panel` in a dock or toolbar, `Surface::Dialog` in a window or
modal, and `Surface::Card` for fields drawn on the canvas. Fills and borders are
derived from that surface and the active theme. Canvas controls retain the card's
zoom-scaled dimensions.

Use the shared 4, 8, and 12 point spacing increments for gaps, control insets, and
separation between layout regions. Control presets use minimum heights of 24
points (compact), 28 points (standard), and 36 points (comfortable), with a
shared 6 point radius. Settings uses compact controls for shortcut editing and
standard controls for forms. Its content uses the dialog layer. Its sidebar and
padded sidebar action area use the panel layer. Light and Dark provide the same
semantic roles.

The shared `orange_ui_controls::color_picker` popup uses a saturation and
brightness square, vertical hue and alpha bars, and editable 8-bit RGB or
RGBA channels. The alpha bar and swatches composite the selected color over
theme-aware checkers with 3-point tiles. The channel controls have equal widths
and the action buttons share a baseline. A selectable text field accepts Hex
and CSS RGB or RGBA.
The format switch chooses which representation to copy. The caller supplies
translated text and owns the color.
Only Apply returns a value. The eight most recently applied colors are shared
across pickers in the current editor session. Color inputs use the same
control on the canvas and in Properties.

The private controls crate registers the bundled Material Symbols font and SVG
loader once per egui context. Use its 16 point icon slot for controls and center
the visible glyph within it. Boolean settings use compact checkboxes with
plain labels. The editor maps graph commands to their own icons and can
override the shared painter. Standard text and monospace use egui's bundled
default fonts across platforms.

| Control | Rule |
| --- | --- |
| Dropdown | Use `Dropdown::show_select` with a centered chevron and a selected value or prompt. A borderless fill distinguishes it from its surface. Use an inset fill on cards and a stronger fill on focus or open. Truncate long selections inside fixed-width fields. |
| Search | Use `controls::search` for dock lists. Inset the field from the dock edge. Center the icon, text, and clear action vertically within the input. Keep hint copy short enough for a sidebar. Clearing the text returns focus to the field. |
| Primary button | Use `Action::Primary` for the main confirmation in a dialog. The selection color supplies its fill and border. |
| Secondary button | Use `Action::Secondary` for ordinary commands with a neutral fill. |
| Outlined button | Use `Action::Outline` for Cancel, Close, and secondary dialog actions. Its resting fill matches the surrounding surface. |
| Destructive button | Use `Action::Destructive` for commands that remove saved data. Pair a restrained error tint with an explicit label. Clear log is a neutral command with a centered trash icon and is disabled for an empty log. |
| Toggle | Use `Action::Toggle` for pressed commands and filters, and the labeled toggle for a Boolean setting. Keep inactive toggles flat. Toggle and segmented controls share the selection fill and focus color. |
| Menu row | Use `menu_item` for context commands. Center icons and text on the same row, align labels to the leading edge, and place shortcut labels at the trailing edge. |
| Toolbar | Use `action_row` when actions precede a search field. Search fills the remaining width or wraps below when the dock is narrow. |
| Form | Use `form_grid` for labels and fields, with shared column and row spacing. Single-line fields center text vertically. |
| Window and popup | Use `dialog_window`, `modal`, or `anchored_popup` for shared surface, radius, border and inset. Keep command routing and close rules with the caller. |
| Keyboard navigation | Use `consume_plain_keys` after deciding which control owns focus. Bindings and conflict rules belong to the editor. |
| Notice | Use `notice` for an inline warning or error, with text that conveys the state. |
| List row | Use the shared raised row surface for clickable search results. The palette owns node names, card previews, and the drag payload. |
| Dialog actions | Put Cancel first, followed by one filled confirmation, in the shared recessed footer. Use the primary color for actions like Install and Save, and the destructive color for Remove. |

Toolbar icon buttons and list rows retain their compact shapes and visible hover
and focus states. When a screen introduces a repeated pattern, define its layout,
spacing, and enabled and focused states in the shared crate before using it
across screens. For changes to a shared component, exercise narrow and wide
layouts, Light and Dark, keyboard focus, and localized text where relevant. Run
the shared-control and editor tests before adopting the change more broadly.

## 5. Variables and connections

* **Variables** filter by name, type or scope. Each scope groups editable names,
  types and initial values, with the current value shown while running. Drag a
  variable onto the canvas for Get or Alt-drag for Set; a variable shared by all
  graphs gets Set shared, the only write the language accepts for it. Each row also
  has Get and Set buttons that place the node in view, and says how many nodes use
  the variable; clicking that selects and shows them. Scopes read *This trigger
  only*, *While the graph runs* and *Shared by all graphs*, and persistence reads
  *Remember after stopping*. The first run of a graph that declares shared variables
  declares them in the host. The node tree also lists
  draggable Get variable and Set variable nodes. Each picks the variable by name and
  type. Declaring from a node opens the same Variables form and selects the new
  variable on the originating node. Shared writes offer only workspace variables.
  Ordinary writes offer local and run variables. On Variable Changed uses an
  initialized run variable with equality support.
* **Types** filters records and enums by name or kind. Larger lists start collapsed
  so their declarations can be scanned before opening individual types.
* **Connections** filter by name or protocol and group by adapter. The connection
  dialog describes each adapter by name and purpose and shows its schema-defined
  settings. Network tuning is under Advanced. The fields scroll independently of
  the save, cancel and settings check actions. Passwords, tokens and keys start
  masked and can be revealed while entering a new value. On save the operating
  system's credential store keeps the secret and the graph keeps its
  `credential://` reference (12, ADR-21). A separate local check reports whether a
  referenced credential is available without opening a connection or approving it.
  Check settings does not read saved credentials. Clicking a connection's usage
  count selects its nodes. Saving edits keeps the form open and reloads the
  persisted settings and credential references. Done closes the form.
* Text, binary and streamed HTTP requests use the same Method dropdown on the
  canvas and in Properties. A wired method is read from its source. HTTP
  Properties places the observed status and response under Results, before
  request setup.
* Multiline inputs use a stable, scrollable area sized to their text. A resize grip
  sets the height in the current editor view. HTTP bodies and headers, network
  messages and Log message use this treatment. Clicking the canvas area enters
  editing without resizing the card. Enter adds a line, Ctrl/Cmd+Enter or clicking
  away commits, and Escape discards the canvas edit.
* **Settings → Extensions → Packages** lists installed and built-in providers with search. Import package
  opens the desktop archive picker, then shows the requested capabilities for
  individual approval. Installing makes the package available to the node tree in
  the same session and keeps active flows pinned to their providers (04).
  Installed versions have a Remove package command with confirmation. Built-in
  providers stay in the list. Removing a version preserves saved provider bindings.
  Removal is blocked while a running or paused graph uses that version, or another
  installed package depends on it. Installed and native service packages have an
  On/Off switch (`Host::set_enabled`, 04). A running or paused graph blocks turning
  off any package whose nodes it uses. Once off, its nodes leave the node tree and
  graphs that use them cannot run. The base and clock nodes are core and have no switch.
  The editor also protects the current open graph while it is stopped. Remove its
  package nodes or close the graph to make those actions available.
* **Access per package.** An installed package that asks for grants shows them as
  boxes in its row; unticking one revokes it for graphs started afterwards
  (`Host::set_access`, 04). Nothing is ticked for the person at import.
* The **Graph bundles** menu under **Settings → Extensions** groups opening and sharing. **Share graph with packages**
  saves the open graph with the stored archives of the
  packages it is locked to as one `.orangebundle` file (`Host::export`); packages
  without a stored archive are named in the Log. **Open shared graph** lists what the
  file carries, marks what is installed, asks for each missing package's grants and
  names what the file lacks. It can install and open, or open without installing; the
  graph opens as a new, unsaved document.
* **A node whose package is missing** keeps its card, settings and wires: its ports
  come from the saved signature, its title tooltip and Properties name the package and
  version, and Properties offers Install package. The node runs once the package is
  installed.
* **Settings → Extensions** includes package management and opens the full-access plugin manager (desktop only).
  It lists native plugin libraries waiting for a
  decision and the ones allowed, with the risk in plain words, and opens at start
  when a new or changed library waits.

## 6. Audio

Connect Audio Source to Frequency Splitter to view its live frequency line. Drag
across the plot and release to add one filtered audio stream output to the card.
Draw across another range to add another output. Each output row has editable
name and frequency fields and a visible delete button. The rows sort by their
lower frequency. The raw spectrum is also available as an output port.
Editing a band preserves its port keys and wires. Removing a band removes its wire connections in one
undoable edit. Drawing on an older spectrum card upgrades it in the same edit
and preserves existing level and execution outputs and their wires.

The plot, bands and ports use the card's zoom scale. The pointer shows the
frequency under it. Collapse Frequency chart to stop painting the plot and
building the trace geometry. The latest display samples remain available when it reopens.
The band controls and filtered outputs stay available. The splitter shares the
EQ gallery’s plot frame, axes and cached trace painter. It supplies logarithmic
analyzer buckets and editable band ranges. The gallery supplies FFT bins and
response curves. Unchanged traces reuse the tessellated mesh.
The spectrum, Audio Level Trigger and connection meters show -120 to 0 dBFS.
The canvas and Properties distinguish disconnected, stopped preview, waiting for samples,
and live input. Properties identifies the connected upstream node and output port.
A connected stopped preview shows the chart or meter scale without audio.
Silence is a valid live reading at the floor.

The Audio group starts with Audio Source, Frequency Splitter, Audio Level Trigger
and Audio Out when browsing or searching for audio. Audio Source uses the full
connection row for its device picker. Searching for Audio In finds Audio Source.
Transient remains searchable for graphs that react to sudden hits, such as kicks.

Connect Audio Source or a filtered band to Audio Level Trigger for an RMS meter in dBFS.
The trigger and gallery use the same segmented RMS widget, including the level
zones, held-peak animation, clipping lights and target marker. A live stereo
input has independent L/R lanes from -120 to 0 dBFS. Mono uses one lane with
the same scale, clip light and threshold handle. For
more than two channels, the display uses aggregate RMS and the highest channel
peak. Each red light latches when a measured sample reaches full scale and
clears when the stream is lost or the analyzer restarts. With a stopped or
unavailable input, the meter shows empty L/R lanes with unlit clip indicators
and keeps the threshold marker in place.
The vertical meter rises immediately, decays smoothly and briefly holds its marker. The Level output shows the current reading beside
the meter. Drag the threshold handle on the left or within the meter, or type a
threshold under the meter. The waveform gallery uses signed min/max columns when
samples outnumber the display pixels. File previews reuse a bounded envelope with
the lowest and highest source-channel sample in each bucket.
The card is built from the definition's control hints (ADR-33): the level-meter visual,
the `aside` row and the `body` fields. Beside the meter sit the outputs and, under
them, Hold and Reset margin as dials side
by side; each dial carries its own name. Under the meter, the stream input, Condition
and Threshold run the full width of the card. In Properties the row label names each
dial. Double-click a dial's value to type.
The meter marker, slider and numeric threshold control mirror the active drag.
Release commits one undoable edit. A typed threshold commits when editing ends.
Threshold, Hold, and Reset margin configure the trigger's subscription.
Hold requires
continuous qualification. Reset margin is the amount the signal must move past the
threshold in the opposite direction to rearm after a firing.
Level and Condition met are live data outputs. Triggered continues execution
after qualification. The threshold, hold and reset always apply to aggregate RMS.
The Channel levels output carries RMS and sample peaks in dBFS and clip flags
in stream channel order, including all channels of a multichannel stream.
Version 1 trigger cards upgrade when a graph opens and keep their settings and
existing wires.

Properties follows the header, aside and body order within the Settings and Inputs
groups. The Advanced group contains fields explicitly placed there. Numeric input
sliders and dials appear next to the typed editor when the input is unwired. Custom
slider marks show guides at the requested values. Dials use their hinted style,
precision and localized caption when provided.

Numeric controls keep non-finite readings out of their painted geometry. A dial
whose range cannot be represented by the rotary control uses a typed field.
A saved node position outside the signed 32-bit coordinate range uses automatic
placement.

Editable duration inputs on the canvas and in Properties show milliseconds and
accept fractional milliseconds up to nanosecond precision. Dragging steps by
whole milliseconds. The graph still stores its duration literals in nanoseconds.

Place Noise Gate between a source and Audio Out, Record to File or another
stream consumer to mute quiet samples. Threshold sets the level to open, Reset
margin sets how far the level must fall to close, and Hold keeps the stream open
through short gaps. Attack fades in and Release fades out. Audio Level Trigger
remains available for flows that need to execute an action at a threshold.

New Audio Out cards play the wired stream while Enabled is on. The inline switch
is available on an unwired input, and a Boolean wire can control the same input.

The source picker separates recording devices, playback capture and current
application sessions. Refresh the list after an application starts playing audio.
Long device names stay on one line. Level and Band Level show a value after the
first measured window. File operations show an editable relative path alongside a
browse button where the operation allows selecting a path. Folder-scoped choices
are stored relative to the connection's root. The standalone connection probe is
available in Connections. See ADR-30 and [05](05-services-and-platforms.md).

## 7. Out of scope for now

Themes beyond light and dark, touch input, collaborative editing.

## 8. Interaction references and review checklist

Properties can open the selected node's existing connection in the same editor
used by Connections or create and bind a profile there. Variables opens a read-only
inspector with full copy for observed text and JSON. Structured value dialogs\nprovide a scrollable JSON field,
a Format JSON action, and localized syntax feedback with a line and column.
The host still checks the value against its declared type on Apply.

| Reference | Rule for OrangeEditor | Review scenario |
|---|---|---|
| [Warudo Blueprint editor](https://docs.warudo.app/docs/blueprints/overview) | Keep graph selection, node discovery, and property editing in distinct places. Preserve the canvas as the main working area. | Find a timer by purpose, drag it to the canvas, then edit its most common input on the card. |
| [Houdini network editor](https://www.sidefx.com/docs/houdini/network/nodes.html) and [Blender 4.5 node editing](https://docs.blender.org/manual/en/4.5/interface/controls/nodes/editing.html) | Offer search at the pointer, show placement before committing, and keep wire insertion reversible as one edit. | Add via keyboard; try valid and invalid wire insertion; undo the successful insertion once. |
| [Unreal Blueprint placement](https://dev.epicgames.com/documentation/unreal-engine/blueprint-basic-user-guide-in-unreal-engine) and [debugger](https://dev.epicgames.com/documentation/en-us/unreal-engine/blueprint-debugger-in-unreal-engine) | Filter by port context when wiring. Distinguish computed preview, observed values, and values that have yet to be produced. | Wire two Bool values to AND and read its unwired output. In a running graph, inspect a pin before and after it executes. |
| [WCAG 2.2](https://www.w3.org/TR/WCAG22/) and [Nielsen Norman Group heuristics](https://www.nngroup.com/articles/ten-usability-heuristics/) | Give icon controls readable names, make keyboard focus visible, and use text or shape alongside color. Keep status and recovery actions close to the work. Use the web criteria as design guidance for the native editor. | Navigate without a mouse. Verify result rows are easy to target, toggle states are distinguishable, and a problem marker leads to explanatory text. |

Before calling a UI change complete, exercise the primary actions in normal and narrow
windows, at default and high display scale, with both pointer and keyboard. Include
long port names and values, a high-density graph, a failed run, and an empty search.
Verify tracing and recording with actual devices if an audio change affects routing.
Headless egui tests cover drawn shapes and synthetic events; visual and hardware checks
need their own evidence.
### Visual hierarchy and feedback

Canvas cards announce their full names when focused. Connector names include the
card, port, direction, and type. Enter or Space on one connector starts a wire.
Activate the opposite end to make the same edit as a pointer drag. Escape or
activating the first connector again cancels the pending wire. Shift+F10 opens
the focused card's or connector's context menu. Audio settings and Boolean
choices expose field names to assistive tools.

- Give the canvas the largest area. Group related controls into stable docks and keep
  the active graph, run state and problems visible. Use a neutral canvas and reserve
  strong colors for selection, execution and faults. Use the same node family cue in
  the palette and card headers when adding family icons. And, Or, Not and Xor share the
  Logic family whatever package provides them. The palette offers And and Or as the
  growable `core/all-conditions` and `core/any-conditions`; the two-input `core/and`
  and `core/or` stay loadable but are not listed (`orange_host::UNLISTED`).
- Keep titles, port labels and values scannable in that order. Prefer one compact
  primary action per card. Use consistent row heights and padding, keep long values
  within the card, and show full precision in the inspector. At distant zoom, keep
  shape and routing readable while removing detail that is too small to read.
- Let settings answer what will change before applying an edit. Confirm destructive
  operations when undo cannot recover the work. Use the same validation wording on
  a node, its inspector and the Problems dock. Preserve the original values and
  wiring when an edit is rejected.
- Keep panel focus and selection visible. Test empty, loading, unavailable and failed
  states alongside the happy path. For network and audio panels, distinguish saved
  settings, connection state and runtime outcomes. Never use credentials in UI
  diagnostics.
- Review dense graph navigation at realistic scale before adding persistent overlays.
  An optional overview is useful when the graph exceeds the viewport. Verify zoom,
  fit and focus behaviour first. [Houdini network navigation](https://www.sidefx.com/docs/houdini/network/navigate.html)

The canvas indexes incoming wires once per frame for displayed values and formulas.
Port value reads borrow their keys. Wire hit testing skips curves whose bounds do not
reach the pointer. Frame timing and interaction latency still require measurement on
dense graphs.

### Portable control composition

Simple fields expose their value, label and focus. An advanced field adds units,
stepping or validation. A composite owns the interaction across its children,
such as a calendar or media transport. The host owns persistent state, translated
labels, media decoding, native pickers and permission lifetimes.

- Give editable fields persistent labels. Icons identify a field purpose and
  supplement its name. Put an actionable error beside its field. Preserve the
  draft across resize, validation and theme changes. Single-line edits can use
  platform IME input. Multiline editors retain vertical, horizontal or two-axis
  resizing on desktop; narrow hosts should choose vertical resize.
- Use 4.5:1 for normal text and 3:1 for large text and essential control
  boundaries. Compare against the actual adjacent fill, including selected and
  hovered states. Pair semantic colors with text or a recognizable symbol.
- Voice previews group play/pause, a seekable waveform and elapsed/duration.
  Equal fader travel represents equal dB increments, with silence at the lower
  stop and unity at the top. The output callback receives linear amplitude.
- Inline video integration reserves a stable aspect-ratio rectangle. Hosts can
  present decoded frames as textures or delegate playback to a native player.
  Route caption selection, full screen, audio focus and app suspension to the
  host. Android Media3 PlayerView and Apple system players are native integration
  options. The desktop WAV gallery demonstrates audio playback.
- A mobile host can render portable egui controls and supply its own picker
  on activation. The optional synchronous desktop file dialog is available on
  Windows, Linux and macOS. Mobile resource handles, including content URIs,
  belong to the host and need not have a filesystem path.
- Spruce and Parchment are original color selections and introduce no external
  theme assets. Theme installation is idempotent for an unchanged configuration.
The gallery reserves the measured space of offscreen catalogue cards without
laying out their previews on steady frames. Width and density changes remeasure
the cards. Horizontal slider thumbs use the same pill silhouette as the notches,
with a larger size to distinguish the active handle.
The gallery offers a left section menu on wide windows and a compact navigation
menu plus bottom shortcuts on narrow windows. Section navigation moves the
single canvas. An ellipsis opens item actions. Search scopes remain optional.
Calendar popups use the same elevated frame, radius and selected foreground as
cards and menus. Semantic note accents follow the frame contour in both densities.

Session tables retain authored order. In sortable tables the active column and
direction are visible, and sorting changes a row-index array only on activation.
Browsed folders load on expansion off the UI thread. Cache directory listings
and failures until refresh or folder replacement. File-name filtering operates
on expanded folders without recursively scanning the filesystem.
Resize affordances occupy the edge that moves: bottom for height, right for
width and the lower-right corner for both axes. Long text scrolls within the
chosen height. A toast may carry supporting text and stay until dismissed.
The shared lifetime policy accepts 2–120 seconds for timed notices. Persistent
gallery errors have a dismissal action and schedule no timer repaints.
Gallery and desktop palette tiles render panels, a slider, and buttons from\nthe shared control colors.
The System tile splits one layout diagonally between light and dark.
Midnight Contrast shares the high-contrast palette checks for text, semantic
inks, boundaries, focus and selected text. Volume uses the themed slider rail
with travel linear in decibels and silence at its lower stop.
Wide settings panes and gallery navigation use full-width rows. Selection is
marked by a leading accent and background tint. Disclosure rows expose
expanded state and group child destinations. Both text and icons remain
visible on the selected surface. A single-axis textarea grip spans 32 points
along its movable edge, and the sort indicator is drawn independently of fonts.
The editor canvas retains one grid mesh per canvas. Viewport bounds, camera,
grid configuration, palette colors and display scale determine reuse. Display
elision measures at most 4096 UTF-8 bytes of a label; copying and inspecting
values keep their original data. The calendar heading centers between equal
navigation targets and opens month and year grids in the same fixed-size popup.
Choosing a year or month moves to the next grid. Only choosing a day commits a date.
On selection or Escape, focus returns to the date field.
