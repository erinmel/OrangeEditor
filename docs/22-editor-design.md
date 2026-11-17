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
| Grid | Faint dots mark the 24-unit snap grid and disappear at distant zoom. Snap is on by default and configurable in Settings. Alt while dragging moves freely. |
| Resize | Resizing the window or a dock preserves the graph point at the center of the canvas. |
| Overlap | Selection, dragging, and completed inline edits bring a card forward. The last active card stays in front when selection clears. Diagnostic marks are painted with their own card. |
| Wires | Execution uses diamond ports and thicker amber wires. Data uses round ports and thinner typed wires. Hovering a port shows its full label and type. A wire can be hovered (it thickens), clicked to select (Delete removes it), and right-clicked for Insert node / Delete. Dragging a node over a wire highlights it; letting go inserts the node between both ends when its ports fit (the model's insertion transaction, 12). |
| Values | Data ports show shortened values inside the card, below their labels. The value inspector shows the full value. A pure node shows its computed result while its inputs are known, even when its output is unwired or an unrelated node is unfinished. |
| Problems | A small red or amber indicator at the top-right of the header reveals details on hover. Faulty ports and wires are marked in the same colour. Clicking a validation marker opens Problems and clicking a runtime failure opens Log. The two views have independent scroll positions. |
| Run feedback | A pulse follows an execution wire when the reported step identifies a route, such as True or False, or a successful completion has one execution output. A failed step retains its diagnostic dot until it succeeds. |
| Groups | Frames are drawn under cards; their title bar carries the collapse arrow and the group's colour. |

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
  schema. Connection references use a dedicated source control. Fields preserve
  their JSON types, and complex configuration remains available in the inspector.
* **Language.** Canvas titles use the same localized catalogue labels as search.
  User-assigned node names are preserved. Port labels use the host's supplied text.
* **Composition.** Embedding applications may call `Editor::ui_with_layout` and
  place or omit the toolbar, status bar, catalogue, inspector, diagnostics and
  editable canvas in their own containers.

## 4. Search and commands

Palette and canvas search use the same full-width result rows. In preview mode the card provides the title and ports; descriptions appear on hover in both result modes. Get variable and enum nodes have illustrative cards until their declarations determine the real ports. Up and Down move the visible highlight, and Enter adds the highlighted result. Dragging a palette result shows a labeled placement preview before dropping it.

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
  Paste, Group, Arrange, Snap), on a card (Rename, Duplicate, Disable, Group, Make
  function, Delete, Help), on a wire (Insert node…, Delete), on a port (Disconnect,
  Promote to variable, Watch value). Every entry names its shortcut.
* The toolbar uses icons with tooltips grouped as File | Edit | Run, from the pinned Material Symbols Rounded library (with its licence recorded in 10), so no symbol depends on system
  fonts.
* The workspace starts with both side docks visible. The bottom bar puts the node dock toggle
  and zoom at the left, the inspector dock toggle at the right, and Problems, Log and
  running status in the center. Properties and Packages share the inspector's top tabs.
  Problems and Log share the same resizable panel. The controls wrap in narrow windows.
* The desktop title follows the current file name. Running, Stopped or No graph
  and the unsaved indicator remain in the status bar.
* Settings is centered and resizable, with a search field, category navigation,
  and a scrolling content pane. It cannot collapse. Shortcut search matches both
  action names and keystrokes.
  Editor command bindings can be recorded, removed and reset. Conflicts are shown
  before assignment. The desktop saves bindings, node language and canvas preferences
  under the user's configuration directory on exit. Graph documents carry none of
  these preferences.
  The desktop applies the draft's blue-gray surface hierarchy. Command icons use
  a consistent 16-point outlined font. File and edit controls stay at the left,
  execution and settings at the right. Run and Stop are separate icon controls
  with state colors on their outlines. Menu and toolbar shortcut labels follow
  the current binding.

## 5. Variables and connections

Implemented 2026-11-16 (FR-114, FR-116); the drag handle and menu use plain text
symbols until the bundled icon font (§4) lands.

* **Variables** list like Blueprint variables: name, type chip, current value while
  running; drag one onto the canvas to get a Get, Alt-drag for a Set; edit in place.
* **Connections** are created from every adapter the host has (network, WebSocket,
  audio, files, any package), listed by name and purpose, not by package ID. Each
  adapter's settings are drawn from its schema; a secret field (password, token, key)
  is entered in a masked field and stored in the operating system's credential store,
  and the document keeps only the `credential://` reference (12, ADR-21). A connection
  shows whether it is in use, by which nodes, and can be tested (open, ping, list
  devices).

## 6. Audio

ADR-30 owns the two-phase audio design. Phase 1 supplies shared device/file sources,
per-node Band level, On band level and On transient settings, source transport and
On finished. Each analyzer names its source through a connection. Signal routing
wires, running-analyzer telemetry, spectra and hit histories belong to phase 2.

The editor uses adapter metadata for device dropdowns on the card. Recording,
playback and playback-capture choices are visually separated. Selecting a device
creates a source declaration when one is needed. Controls use the first unused
input row on the left, or a new row when both sides are full. Scalar configuration
fields render from the node's schema below its ports, including detector bands,
thresholds and timing. Complex source settings remain in the inspector.
Stream references show their readable type without process-local record IDs.
Pure authoring values remain on the cards when Run starts until observed values
replace them.

Opening the configuration window does not open an audio probe. The explicit
connection test in the Connections tab remains available. A monitor of the actual
running analyzer requires a host telemetry API. The UI must identify any standalone
connection probe as a device test.

Phase 2 presentation will show a spectrum with a selected Hz range, threshold and
release lines, measured level, and hit markers. A continuous level output supports
animation modulation. Threshold crossings and transient events support one-shot
effects. Preview and Listen are separate controls with explicit activation. The
runtime decisions and migration belong to ADR-30.

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
