# Repeat an action with a loop

In this tutorial you calculate 3 to the power of 4 by multiplying a variable by 3, four
times. You meet the [**Count**](../nodes/core/count/1/) loop, which runs the same
steps a fixed number of times, and you see data wires and execution wires work
together in one flow. It takes about ten minutes.

You should know how to place nodes, wire them and fire a trigger. If you do not, start
with [Count button presses](../count-button-presses/).

## Declare the variable

1. Open the editor with an empty graph:

   ```sh
   cargo run -p orange-desktop --locked
   ```

2. Open **Variables** and add a variable named `power` with the type **Int**. Keep its
   scope as a run variable.

## Start from one

Each run must start from the same value, or a second press would keep multiplying the
last result.

1. Place [**On manual**](../nodes/core/on-manual/1/) and name it `Calculate`.
2. Place [**Set variable**](../nodes/core/set-variable/1/) to its right. In
   **Properties**, choose `power` and set the value to `1`.
3. Wire the trigger's **Exit** to the action's **Enter**.

## Add the loop

1. Place [**Count**](../nodes/core/count/1/) to the right of **Set variable**.
2. In **Properties**, set [**count**](../nodes/core/count/1/#port-count) to `4`.
3. Wire **Set variable**'s **Exit** to **Count**'s **Enter**.

**Count** has two execution outputs. [**body**](../nodes/core/count/1/#port-body)
runs once per pass, and [**completed**](../nodes/core/count/1/#port-completed) runs
after the last pass. Its [**index**](../nodes/core/count/1/#port-index) output says
which pass is running, starting at `0`.

## Multiply on each pass

The step that runs on each pass reads `power`, multiplies it by 3 and writes it back.

1. Place [**Get variable**](../nodes/core/get-variable/1/) below the loop and choose
   `power` in **Properties**. It has no execution ports: it is read when a step needs
   its value.
2. Place [**Multiply**](../nodes/org.orange.math/multiply/1/). Wire **Get variable**'s
   [**value**](../nodes/core/get-variable/1/#port-value) to its
   [**a**](../nodes/org.orange.math/multiply/1/#port-a), and set
   [**b**](../nodes/org.orange.math/multiply/1/#port-b) to `3` in **Properties**.
3. Place a second **Set variable** with `power`, and wire **Multiply**'s
   [**product**](../nodes/org.orange.math/multiply/1/#port-product) to its **value**.
4. Wire **Count**'s **body** to this **Set variable**'s **Enter**.

<!-- tutorial-graph: power-with-count -->

The example graph keeps the base and the exponent in **Int** nodes so you can change
them in one place, and saves the answer in a second variable on **completed**.

## Run it

1. Check that **Problems** is empty, then press **Run**.
2. Press **Fire** on `Calculate`.
3. Open **Variables** and confirm **`power = 81`**.

Press **Fire** again: the answer is still 81, because the first **Set variable** starts
each activation from 1.

## What you built

- **A loop is an action with more than one exit.** **body** runs for each pass and
  **completed** once at the end, so the steps after the loop wait for it.
- **Data is pulled when needed.** **Get variable** and **Multiply** run on each pass,
  when the **Set variable** on **body** asks for its value, so they always see the
  latest `power`.
- **Literals fill unwired inputs.** An input you set in **Properties** keeps its value
  until you wire something to it.

Next, [run this kind of graph from your own Rust program](../run-a-graph-from-rust/),
or read why graphs have [two kinds of wires](../data-and-execution/).
