# Types and conversions

Every value in an OrangeEditor graph has an exact type, and a wire connects only
ports whose types match. When you need a different type, you add a conversion node
you can see.

## Exact types

- **Built-in types** include whole numbers, decimal numbers, true-or-false, text,
  bytes, durations, timestamps, lists, maps, options, results and JSON.
- **Your own types** are records and enums you declare in **Types**.
- **There is no "any" type.** A port that accepts several types is generic: its type
  settles from what you wire to it, and the card shows the type it settled on.

## Why there are no hidden conversions

Many node editors turn one type into another silently when you connect them, and
lose data along the way: a decimal becomes a whole number and its fraction is gone, or
a large number wraps around.

OrangeEditor refuses the wire instead and offers a conversion. Because the conversion
is a node on the canvas:

- you can see where a value changes type;
- it states how it handles values that do not fit, such as rounding, clamping or
  failing;
- a conversion that can fail has a **failed** route you can handle.

## Adding a conversion

- Drop a wire from an output onto an input of another type: the editor offers the
  conversions between them.
- Or search the **Conversions** group in **Nodes**, for example `whole number text`.

## Options and results

Some values may be missing or may be an error, and their types say so:

- An **option** is a value or nothing. **Coalesce** gives a fallback for nothing.
- A **result** is a value or an error. **Match result** routes execution to **ok** or
  **err** with the value or the error.

## See also

- [Data wires and execution wires](../data-and-execution/)
- [Graph language reference](../reference/graph-language/), the exact type rules
