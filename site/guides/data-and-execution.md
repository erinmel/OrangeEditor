# Data wires and execution wires

An OrangeEditor graph has two kinds of wires. Knowing which one you are drawing
explains almost every question about when something runs.

## Execution wires: what happens next

Execution wires join an action's outcome, such as **then** or **failed**, to the
**in** port of the next action. They are drawn as plain arrows between actions.

- Work starts only at an **entry**: a trigger such as **On manual**, **On timer** or
  **On file change**. Each time it fires, it starts an *activation*.
- Execution follows the wires from action to action, one step at a time.
- An action with several outcomes continues on exactly one of them. **Branch** goes to
  **true** or **false**; **HTTP request** goes to **then** or **failed**.

Effects, the things that change the world or the graph's state, happen only where
execution reaches. A **Set variable** that no execution wire reaches never runs.

## Data wires: where a value comes from

Data wires carry a typed value from an output to an input. They are coloured by type.

- A value is **pulled** when an action that execution reached needs it. Nothing is
  computed ahead of time.
- **Pure** nodes, such as **Add** or **JSON parse**, compute from their inputs only.
  A pure value is computed once per demand, however many inputs read it.
- An output that belongs to an outcome, such as an HTTP **status**, is available only
  after execution passed through that outcome.

## Putting them together

In the [counter tutorial](../count-button-presses/), execution runs from
**On manual** to **Update variable**. If you wired a **Get variable** into a **Branch**
condition, its value would be pulled at the moment execution reached the branch: the
current value then, not when the graph started.

## Why two kinds

Keeping them apart makes order visible. You can read from the execution wires exactly
which effects happen and in which order, while data wires stay free of side effects,
so a value never runs an effect by being read.

## See also

- [Types and conversions](../types-and-conversions/)
- [Graph language reference](../reference/graph-language/), the exact rules
