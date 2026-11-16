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
   field) with a small sign; detail is on hover and in the Problems list. Nothing is
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
| Grid | Snap to grid is on by default (24 units), shown in the toolbar; Alt while dragging moves freely. |
| Wires | A wire can be hovered (it thickens), clicked to select (Delete removes it), and right-clicked for Insert node / Delete. Dragging a node over a wire highlights it; letting go inserts the node between both ends when its ports fit (the model's insertion transaction, 12). |
| Values | Each data port shows the value it last carried in a small pill outside the card; hovering shows it whole; clicking pins it open. A pure node shows its result live while its inputs are known (preview, compiled with the same plan rules as a run). |
| Problems | A small red or amber dot on the header's left with a count; the port or wire at fault is ringed in the same colour. No outline around the whole card. The dot's tooltip lists the problems; clicking opens Problems at them. |
| Run feedback | A brief pulse along the card's top edge when a step runs; a failed step keeps a red top edge and the dot until it succeeds. |
| Groups | Frames are drawn under cards; their title bar carries the collapse arrow and the group's colour. |

## 3. Cards

* **Inline values.** A literal input is edited on the card: a single click on a number
  types it, a drag scrubs it; text is typed in place; a choice is a dropdown; a path
  opens the system's file or folder chooser on click.
* **Expressions.** Operators show what they compute under the title with their inputs'
  names or values: Greater shows `a > b`, or `12 > 7` when both are known. Comparison,
  arithmetic and logic nodes get this from their definition's `formula` presentation hint.
* **Triggers.** A trigger shows its contents: On manual has a Fire button always present
  (disabled, with the reason on hover, while the graph is not running) and its payload
  field; On timer shows its interval; an event trigger shows the event it waits for.
* **Value nodes** (Int, Float, Bool, Text) are compact: one row, the value itself.

## 4. Search and commands

* One search for the palette, the wire-drop list and the quick-add box. It matches
  names, aliases, categories, concepts and the words of each node's description, in the
  active language and in English. Concepts group nodes by purpose: *trigger / when /
  start* finds every entry kind (On manual, On timer, On start, On event, On level);
  *math*, *compare*, *text*, *time*, *audio*, *network*, *file*, *variable*, *loop*.
* Results rank: exact name, name start, word start, alias, concept, description, fuzzy
  (one typo). Each result shows its category and a one-line purpose; arrow keys move,
  Enter adds.
* Concepts and aliases are translatable catalogue entries (13): a package may add its
  own, and a locale adds words without changing IDs.
* The right-click menu is the command centre: on the canvas (Add node… with search,
  Paste, Group, Arrange, Snap), on a card (Rename, Duplicate, Disable, Group, Make
  function, Delete, Help), on a wire (Insert node…, Delete), on a port (Disconnect,
  Promote to variable, Watch value). Every entry names its shortcut.
* The toolbar uses icons with tooltips grouped as File | Edit | Run | View, from one
  bundled icon font (with its licence recorded in 10), so no symbol depends on system
  fonts.

## 5. Variables and connections

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

Audio moves from level measurement to signal routing:

* **Audio in** (device): any capture endpoint, a microphone, a line input, a virtual
  cable, or the system's playback through loopback where the OS offers it. Its output is
  an audio stream; the card shows a live meter and, expanded, a waveform.
* **Audio out** (device): plays an audio stream on a chosen endpoint.
* **Level**, **Gate / threshold**, **Record to file**, **Play file**, **Mix** operate on
  streams. "When the microphone hears sound" becomes *On level* over any Audio in.
* Streams are a typed resource (11 §9 streams, 05); callbacks stay allocation-free
  (AGENTS.md), and endpoint behaviour is validated per OS, never inferred.
* Seek is renamed *Jump to time* and explained on its card; it moves every sound
  playing on an output to a position.

This section needs language and runtime work: an audio stream type, its bounded buffers
and per-OS capture validation (05, V02/V07/V09/V10).

## 7. Out of scope for now

Themes beyond light and dark, touch input, collaborative editing.
