# Troubleshooting

Find the symptom, then follow the fix.

## The editor or download

### Windows says it protected your PC.

Preview builds are not code-signed yet. Choose **More info**, then **Run anyway**.
Check the download against `SHA256SUMS` on the release page first.

### macOS says it cannot check the app.

Right-click OrangeEditor and choose **Open**, or allow it in **System Settings →
Privacy & Security**.

### A Linux source build fails on ALSA.

Install the ALSA development headers, such as `libasound2-dev` on Ubuntu and
Debian.

## Running a graph

### Run is blocked.

Open **Problems** and follow the first diagnostic to the card, port or package it
names.

### Nothing happens after Run.

A graph starts work only at an entry. Press **Fire** on an On manual, or cause the
event its entry waits for, then follow its execution outputs.

### An action never runs.

Check that an execution wire reaches its **in** port. An action without one never
runs; see [Data wires and execution wires](../data-and-execution/).

### A trigger faults when pressed quickly.

Its queue is full. A trigger keeps up to 16 activations waiting while an earlier one
runs. Give each activation less to do, or change `capacity` and `overflow` in the
node's `activation` in the graph file; see the
[graph file format](../reference/graph-file-format/).

### Two ports will not connect.

Their types differ. Drop the wire on the input to see the conversions, or search
**Conversions** in **Nodes**.

## Connections, files and the network

### The editor asks to approve a connection.

A graph is using a connection that reaches outside the computer for the first time.
Approve it if you trust the graph.

### An HTTP request goes to failed.

No answer arrived: the address is wrong, the service is down, or it timed out. An
error status such as `404` arrives on **then** instead; read **status**.

### A file watcher reports a missing folder.

The watched folder does not exist yet. Fix the path, or set **wait_for_folder** to
wait until it appears.

## Packages

### A saved node has a missing provider.

The graph needs a package you do not have. Review its package and version in
**Packages**, then install that exact package.

### A plugin's request is not allowed.

It lacks the grant for that address. Review its grants in **Packages**: **http**
reaches public `https://` addresses, **http-local** this computer and the local
network.

## Keys and devices

### A key set to Anywhere works only while the editor is focused.

Another application or the system already uses that key, or your Linux desktop
offers no global shortcuts. The Log names the key; choose another one.

### An audio device is missing.

Connect the device, then choose it again in its connection. The Log says which device it could not open.

## Still stuck

Search this documentation, or open an issue on
[GitHub](https://github.com/erinmel/OrangeEditor/issues) with the Log and the graph,
leaving out any secrets.
