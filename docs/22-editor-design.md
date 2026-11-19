# Editor design direction

How the OrangeEditor canvas, panels and node families should look and behave. It is the
reference for editor work after 2026-11-16; earlier editor notes (02, 21) describe what
exists. It draws on node editors people already know (Blender's node editors, Unreal
Blueprints, Houdini, ComfyUI, Warudo) and on the owner's review of the current editor.
It changes no graph semantics; where a change needs one (11), it is called out.

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
| Grid | A magnet icon toggles Snap to grid. Grid settings uses one segmented control for Dots, Lines and Hidden, and a slider for 12, 24, 36 or 48 graph units. The same controls are available in Display and canvas. Grid visibility is independent of snapping. The default spacing is 24 graph units. Fine grids disappear at distant zoom. Alt while dragging moves freely. The zoom percentage opens Fit and Actual size. |
| Resize | The desktop uses the platform window frame for resizing. Resizing the window or a dock preserves the graph point at the center of the canvas. |
| Overlap | Selection, dragging, and completed inline edits bring a card forward. The last active card stays in front when selection clears. A front card covers only the area it overlaps and receives pointer input there. Exposed controls on rear cards stay visible and usable. |
| Wires | Ports are round. Execution uses amber ports and thicker amber wires. Data uses typed colors. Hovering a port shows its full label and type. A wire can be hovered (it thickens), clicked to select (Delete removes it), and right-clicked for Insert node / Delete. Dragging a node over a wire highlights it; letting go inserts the node between both ends when its ports fit (the model's insertion transaction, 12). |
| Values | Data ports show compact previews inside the card, with room reserved for the port labels. Clicking a preview or its label opens a scrollable, read-only inspector. Copy includes the complete observed text or JSON even when the preview is shortened. The editor previews pure outputs whose authoring inputs are known and shows completed effect outputs even without outgoing wires. |
| Problems | A small red or amber indicator at the top-right of the header reveals details on hover. Faulty ports and wires are marked in the same colour. Clicking a validation marker opens Problems and clicking a runtime failure opens Log. The two views have independent search, filters and scroll positions. |
| Run feedback | The reported execution route glows along the whole wire and fades in 160 ms. A stream wire with a current runtime sample shows small moving dots. A failed step retains its diagnostic dot until it succeeds. |
| Groups | Frames are drawn under cards and move by their title bars, including when collapsed. Snapping moves every member by the same offset and anchors the frame at its upper-left corner. Clicking a collapsed frame selects it for Ungroup. |

## 3. Cards

* **Inline values.** A literal input is edited on the card: a single click on a number
  types it, a drag scrubs it; text is typed in place; a choice is a dropdown; a path
  opens the system's file or folder chooser on click.
* **Expressions.** Operators show what they compute in a dedicated body row with their inputs'
  names or values: Greater shows `a > b`, or `12 > 7` when both are known. Comparison,
  arithmetic and logic nodes get this from their definition's `formula` presentation hint.
* **Triggers.** A trigger shows its contents: On manual has a Fire button in the body
  (disabled, with the reason on hover, while the graph is not running) and its payload
  field; On timer shows its interval; an event trigger shows the event it waits for.
* **Value nodes** (Int, Float, Bool, Text) are compact: one row, the value itself.
* **Arithmetic.** Add and other arithmetic cards use their named ports and values.
  Comparison cards may retain a comparison expression. Ordinary arithmetic has
  no redundant expression footer.
* **Configuration templates.** Scalar controls are drawn from each definition's
  schema. Network connection references are dropdowns of compatible saved
  profiles and display the selected name. Profile editing lives in Connections.
  Device nodes have an inline device selector. Fields preserve their JSON types.
* **Language.** Canvas titles use the same localized catalogue labels as search.
  User-assigned node names are preserved. Port labels use the host's supplied text.
* **Composition.** Embedding applications may call `Editor::ui_with_layout` and
  place or omit the toolbar, status bar, catalogue, inspector, diagnostics and
  editable canvas in their own containers.

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
* **Every editable value is a field block**: its name on one line, the control on the
  next, full width. The input's dot sits on the name line. This holds for inputs and
  for settings alike, so a person never needs to know which is which.
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
* **No menus on cards.** Everything a card shows is edited on the card; there is no
  ⋯ menu or popover. Tuning settings (numeric policies, analysis window) are not on
  the card at all: they live in Properties under Advanced.
* **No redundant words.** No type labels (hover shows the type); a field's name is
  left out when it only repeats the card's title (Audio source's device list, Get
  variable's variable).
* **Problems stay with the card.** A node's problem is an icon and a thin accent at
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

`orange-desktop` has an ignored test that writes one graph per package with every
searchable node (`ORANGE_GALLERY_DIR=<folder> cargo test -p orange-desktop
package_galleries -- --ignored`); open each with `--canvas-only --screenshot` and
check every card against these rules after a rendering change.
* **Audio charts.** Frequency Splitter reserves a collapsible plot above its port
  rows. Audio Level Trigger places its meter beside its outputs and settings.

## 4. Search and commands

Palette and canvas search use the same full-width name rows. Purpose and category
appear together on hover. Up and Down move the highlight, and Enter adds the
highlighted result. Dragging a result shows a placement preview under the pointer.

* One search for the palette, the wire-drop list and the quick-add box. It matches
  names, aliases, categories, concepts and the words of each node's description, in the
  active language and in English. Concepts group nodes by purpose: *trigger / when /
  start* finds every entry kind (On manual, On timer, On start, On event, On level);
  *math*, *compare*, *text*, *time*, *audio*, *network*, *file*, *variable*, *loop*.
* Results rank: exact name, name start, word start, alias, concept, description, fuzzy
  (one typo). Each result has a category and a purpose available on hover; arrow keys move,
  Enter adds.
* Concepts and aliases are translatable catalogue entries (13): a package may add its
  own, and a locale adds words without changing IDs.
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
* Settings uses a centered, resizable dialog with a shared search field, an
  independently scrolling content pane and a fixed Done button. General groups Language
  and Node search. Node search displays either names or read-only canvas cards with
  their ports and charts directly in the results. The same choice applies to the node
  tree and canvas quick-add. The section
  list becomes a selector in compact windows. Search finds settings sections, shortcut
  actions and key combinations. Display and canvas groups controls by interface,
  panels and canvas. Interface scale applies when a percentage is chosen in
  its dropdown. At large scales or narrow widths, the editing commands can scroll
  horizontally so Settings and the run controls stay visible.
  The Keyboard shortcuts page displays grouped, left-aligned action labels and
  single-line bindings. Select a binding to type
  or record a replacement. Shortcuts can be removed or reset, and conflicts require
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
* Problems shows errors and warnings with icons, counts and filters alongside search.
  A red error
  blocks execution. An amber warning is advisory. A graph with other usable nodes
  may omit isolated incomplete nodes from its execution snapshot and show their
  diagnostics as warnings. A standalone unreferenced value or action can also stay
  out of the execution snapshot. Graph exports count as consumers. A Frequency
  Splitter with no input audio stays idle and
  warns even when it is the only card. If omitting it leaves a consumer without a
  required input, Problems reports the consumer's error. Wiring or repairing an
  omitted node makes it eligible for the next validated live update. Rejected live
  updates keep the last validated snapshot.

### 4.1 Log

The Log has text search and level filters for CRITICAL, ERROR, WARN, INFO, DEBUG,
TRACE and All. It shows run lifecycle, runtime faults and messages written by the
graph. Successful steps and live values are recorded at TRACE only when Live trace
is on. Consecutive graph messages with the same text and level share a line and
a count. At most 256 graph messages wait for the UI, and excess messages are counted.

New **Log message** nodes use definition version 2. Level defaults to INFO. Add up
to 24 Argument input ports in slot order, then use sequential braces or zero-based
numbered braces in Message. Doubled braces write literal braces. Unknown placeholders
stay visible. Arguments accept text, including values formatted by explicit conversion
nodes. A message is bounded at 16 KiB at a valid UTF-8 boundary. The saved version 1
definition retains its original severity choices and literal-message behavior.

## 5. Variables and connections

* **Variables** filter by name, type or scope. Each scope groups editable names,
  types and initial values, with the current value shown while running. Drag a
  variable onto the canvas for Get or Alt-drag for Set. The node tree also lists
  draggable Get variable and Set variable nodes. Each picks the variable by name and
  type. Declaring from a node opens the same Variables form and selects the new
  variable on the originating node.
* **Types** filters records and enums by name or kind. Larger lists start collapsed
  so their declarations can be scanned before opening individual types.
* **Connections** filter by name or protocol and group by adapter. They use the adapters the host has (network, WebSocket,
  audio, files, any package), listed by name and purpose, not by package ID. Each
  adapter's settings are drawn from its schema; a secret field (password, token, key)
  is entered in a masked field and stored in the operating system's credential store,
  and the document keeps only the `credential://` reference (12, ADR-21). A connection
  shows whether it is in use, by which nodes, and can be tested (open, ping, list
  devices). Saving edits to an existing connection keeps its form open and
  reloads the persisted values and credential references. Done closes the form.
* Text, binary and streamed HTTP requests use the same Method dropdown on the
  canvas and in Properties. A wired method is read from its source. HTTP
  Properties places the observed status and response under Results, before
  request setup.
* Multiline inputs use a stable, scrollable area sized to their text. A resize grip
  sets the height in the current editor view. HTTP bodies and headers, network
  messages and Log message use this treatment. Clicking the canvas area enters
  editing without resizing the card. Enter adds a line, Ctrl/Cmd+Enter or clicking
  away commits, and Escape discards the canvas edit.
* **Packages** lists installed and built-in providers with search. Import package
  opens the desktop archive picker, then shows the requested capabilities for
  individual approval. Installing makes the package available to the node tree in
  the same session and keeps active flows pinned to their providers (04).

## 6. Audio

Connect Audio Source to Frequency Splitter to view its live frequency line. Drag
across the plot and release to add one filtered audio stream output to the card.
Draw across another range to add another output. Each output row has editable
name and frequency fields and a visible delete button. The rows sort by their
lower frequency. Editing a band preserves its port keys and wires. Removing a band removes its wire connections in one
undoable edit. Drawing on an older spectrum card upgrades it in the same edit
and preserves existing level and execution outputs and their wires.

The plot, bands and ports use the card's zoom scale. The pointer shows the
frequency under it. Collapse Frequency chart to stop painting the plot and
preparing new display samples. The band controls and filtered outputs stay available.
The spectrum, Audio Level Trigger and connection meters show -120 to 0 dBFS.

The Audio group starts with Audio Source, Frequency Splitter, Audio Level Trigger
and Audio Out when browsing or searching for audio. Audio Source uses the full
connection row for its device picker. Searching for Audio In finds Audio Source.
Transient remains searchable for graphs that react to sudden hits, such as kicks.

Connect Audio Source or a filtered band to Audio Level Trigger for an RMS meter in dBFS.
The vertical green, yellow and red meter rises immediately, decays smoothly and
briefly holds the recent peak. The Level output shows the current reading beside
the meter. Drag the threshold line or type a threshold in the settings column.
The direction and hold duration sit in the same column.
Level and Condition met are live data outputs. Triggered continues execution
after qualification.

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

- Give the canvas the largest area. Group related controls into stable docks and keep
  the active graph, run state and problems visible. Use a neutral canvas and reserve
  strong colors for selection, execution and faults. Use the same node family cue in
  the palette and card headers when adding family icons.
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
