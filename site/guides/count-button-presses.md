# Count button presses

In this tutorial you build a small graph from an empty canvas: three buttons that
add one, subtract one and reset a counter. Along the way you meet the pieces every
graph uses: a trigger that starts work, a variable that keeps state, and actions
joined by execution wires. It takes about ten minutes.

The finished graph is the
[counter-with-reset example](../examples/#counter-with-reset), so you can compare your work with it
at any point.

## Open an empty graph

Start the editor from the repository root:

```sh
cargo run -p orange-desktop --locked
```

The editor opens with an empty graph. Keep the **Nodes** panel open on the left and
**Properties** on the right.

## Declare the counter

A variable holds a value between activations of the graph.

1. Open **Variables** and add a variable named `Counter`.
2. Give it the type **Whole number** and the start value `0`.
3. Keep its scope as a run variable: it lives while the graph runs and starts again at
   `0` the next time you press **Run**.

## Add the first button

1. Search **Nodes** for `on manual` and place [**On manual**](../nodes/core/on-manual/1/). This trigger starts an
   activation each time you press **Fire** on it while the graph runs.
2. Name it `Increment` in **Properties**, so you can tell the three buttons apart.
3. Place [**Update variable**](../nodes/core/update-variable/1/) to its right.
4. In **Properties**, choose `Counter` as its variable and **Add** as its operation,
   and set its value to `1`.
5. Drag from the trigger's [**Exit**](../nodes/core/on-manual/1/#port-then) to the
   action's [**Enter**](../nodes/core/update-variable/1/#port-in). This execution wire
   tells the editor which action runs next.

<!-- counter-wire -->

## Run it

1. Check that **Problems** is empty, then press **Run**.
2. Press **Fire** on `Increment` three times.
3. Open **Variables** and confirm **`Counter = 3`**.

Each press started its own activation, and they run in the order you pressed. A
trigger queues up to eight presses that are still waiting; you can change how many,
and what happens to more, in its **Properties**.

## Add the other two buttons

Stop the graph, then add:

- A second **On manual** named `Decrement`, wired to an **Update variable** with
  `Counter`, **Subtract** and the value `1`.
- A third **On manual** named `Reset`, wired to [**Set variable**](../nodes/core/set-variable/1/) with `Counter` and
  the value `0`.

Run the graph again and try all three. Then use **Save as** to keep your graph.

<!-- tutorial-graph: counter-with-reset -->

## What you built

- **Triggers start work.** Nothing runs until an entry node such as **On manual**
  fires.
- **Execution wires order actions.** An action runs when execution reaches **Enter**
  and continues from the appropriate output.
- **Variables keep state.** Actions change them; the value survives between
  activations of the same run.

Next, learn why graphs also have [data wires](../data-and-execution/), or
[call an HTTP API](../call-an-http-api/) from a button.

## Try a graph with data wires

Open [JSON extraction](../examples/#json-extraction) and fire **Read sample JSON**.
[**JSON Parse**](../nodes/org.orange.json/parse/1/) reads the sample, then
[**JSON Read Text**](../nodes/org.orange.json/read-text/1/) supplies the name
or its fallback to **Set variable**. Look at **Variables** after firing.
The value of `extracted_text` should be `Studio mic`.

<!-- tutorial-graph: json-extraction -->
