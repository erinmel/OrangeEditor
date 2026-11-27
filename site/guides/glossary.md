# Glossary

The words this documentation uses, in alphabetical order.

### Activation

One run of a flow from an entry, started each time the entry fires. Several
activations can wait in the entry's queue.

### Bridge

A Bridge Out and Bridge In pair that shares a value without a visible wire. Reading
the Bridge In pulls the value; it never starts execution.

### Channel

The name Dispatch and Listen share. A Dispatch on a channel starts every Listen on
it.

### Connection

A configured link to something outside the graph, such as an HTTP service, a folder
or an audio device. It holds the address and credential references.

### Conversion

A node that changes a value from one type to another, with its rules for values that
do not fit. Conversions are never hidden.

### Credential reference

A `credential://` link to a secret in the operating system's credential store.
Graphs keep references, never secrets.

### Data wire

A wire that carries a typed value from an output to an input. Values are pulled
when needed. See [Data wires and execution wires](../data-and-execution/).

### Definition

The description of a node kind: its ID, version, ports and settings.

### Entry

A node that starts activations, such as On manual, On timer or Listen.

### Execution wire

A wire that says which action runs next.

### Flow

A graph while it runs, on its own thread. One host runs many flows.

### Grant

A permission you give a Wasm plugin: files, http, http-local or compute.

### Graph

A document of nodes and wires, saved as JSON.

### Host

The `orange-host` session an application uses to edit, check and run graphs.

### Node kind

A kind of node, such as Branch or HTTP request, identified by an ID such as
`core/branch`.

### Option

A value that may be missing.

### Package

An installable bundle of node kinds, graphs or code, in an `.orangeplug` archive.

### Provider

The exact package and version that implements a node kind in a saved graph.

### Pure node

A node that computes from its inputs only, without effects. Its value is computed
once per demand.

### Result

A value or an error.

### Run variable

A variable that lives while a flow runs and starts again when it runs next.

### Shared variable

A variable every graph on a host can read and write.

### Workspace event

An event one graph dispatches to the other running graphs on the same host.
