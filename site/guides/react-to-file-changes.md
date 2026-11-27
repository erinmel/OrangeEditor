# React to file changes

This guide starts a flow whenever something changes in a folder: a file appears, is
edited, is deleted or is renamed.

## Grant a folder

A graph reaches files only through a folder connection, and only inside that folder.

1. Open **Connections** and add a **Folder** connection.
2. Choose the folder to watch. Paths that leave it, links that point outside it and
   reserved Windows names such as `CON` are refused.

## Start a flow on each change

1. Place **On file change** and choose the folder connection in **Properties**.
2. To watch only part of the folder, set its **folder** input to a path inside it,
   such as `incoming`.
3. Wire its **then** to the actions that should run for each change.

Each change starts its own activation with a **payload** record that holds:

- the kind of change: created, changed, deleted or renamed;
- the path, relative to the granted folder;
- the previous path, for a rename;
- whether the item is a folder.

<!-- tutorial-graph: editor-file-trigger file-change-journal -->

## When the folder does not exist yet

By default a missing watched folder is reported as a fault, so you notice a wrong
path. If the folder is created later, for example by another program, set
**wait_for_folder** to true: the trigger waits quietly and starts watching once the
folder appears.

Files already in the folder when watching starts are its starting point and do not
fire. Only later changes do.

## Act on the file

Use the file nodes in the same category to read, copy, move or delete what changed.
They work through the same folder connection, so they stay inside the granted folder
too.

The [file change journal](../examples/#file-change-journal) reads **path** from the
payload with [**Get field**](../nodes/org.orange.collections/get-field/1/), ends it
with a line break and appends it to `changes.log`:

<!-- tutorial-graph: file-change-journal -->

## See also

- [Node reference: Files](../nodes/)
- [Count button presses](../count-button-presses/) for the basics of triggers and actions
