# Security model

OrangeEditor runs graphs and plugins that other people write, and those graphs reach
files, the network and your devices. This page explains what each kind of code may
do, what stops it doing more, and what is up to you.

## Who can do what

| Code | Runs | Limited by |
|---|---|---|
| Built-in nodes | Inside the application | Reviewed with the application |
| Graphs | In the runtime | Their nodes, and limits on time, memory, queues and recursion |
| Wasm plugins | Inside the application, in a sandbox | Grants you give at install, and fuel, time and memory limits |
| Native plugins | Inside the application | Nothing: they act with all of your rights |

## Graphs

A graph can only do what its nodes do. A graph you open does not run until you press
**Run**, and it reaches the outside world only through connections:

- **Connections hold addresses and credentials**, so a graph file never contains them.
  The first time a graph uses a connection that reaches outside the computer, the
  editor asks you to approve that graph for it.
- **Folders are granted, not paths.** File nodes work inside a folder connection only.
  Paths that climb out of it, links that point outside it, Windows device names such
  as `CON`, and alternate data streams are refused, and the folders on the path are
  held open so they cannot be swapped during an operation.
- **HTTP requests** reach only the base address of their connection. Redirects are not
  followed, and responses are size-limited.

## Credentials

Secrets such as API keys and tokens live in the operating system's credential store.
Graphs, exports, logs and traces hold only `credential://` references and redacted
values, so sharing a graph never shares its secrets.

## Wasm plugins

A Wasm plugin runs inside a sandbox with no access to the computer beyond what you
grant when you install it:

- **files**: its own data folder, with a size quota.
- **http**: public `https://` addresses, optionally only the hosts you list. It never
  reaches this computer or the local network: the address is checked again after the
  name is looked up, so a public name cannot point it inward.
- **http-local**: this computer and the local network, such as a device on your Wi-Fi.
- **compute**: a larger time and memory budget for heavy work.

You can see and revoke grants in **Packages**. Each call runs with limited fuel, time
and memory; a plugin that exceeds them faults its own activation and never stalls the
editor.

## Native plugins

A native plugin is a library that runs with all of your rights: it can read and change
your files, use the network and devices, read your credentials and crash the
application. The desktop loads one only after you allow full access to that exact
file, and asks again if the file changes. Install native plugins only from people you
trust.

## Packages and updates

- A package's files are checked against the digest recorded in it, and a graph
  records the exact provider it was built with, so a package cannot change under a
  saved graph.
- An update feed can be signed. A signed feed is checked against the publisher's key
  before it is read, and `orange updates --signed-only` refuses unsigned ones.

## What is up to you

- Install native plugins only from trusted sources.
- Grant Wasm plugins only what they need, and prefer a host list for **http**.
- Keep the application updated, and check downloads against the published
  `SHA256SUMS` and provenance attestation.

## See also

- [Create packages](../packages/)
- [Package and permission contract](../reference/packages/)
