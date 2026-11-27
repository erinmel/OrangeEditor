export const ui = {
  contents: 'On this page', previous: 'Previous', next: 'Next', sequence: 'Continue reading',
  skip: 'Skip to content', search: 'Search documentation', theme: 'Colour theme',
  themes: ['Automatic', 'Light', 'Dark', 'High contrast'],
  docs: 'Documentation', revision: 'Source revision', index: 'Plain-text index',
  language: 'Language', searchShort: 'Search', searchAll: 'All', searchGuides: 'Guides', searchNodes: 'Nodes',
  close: 'Close', placeholder: 'Search pages, nodes, or IDs.',
};

export const tutorial = {
  'counter-first-step': ['Increment and Add one cards connected in OrangeEditor', 'The execution wire runs from Increment to Add one.'],
  'counter-with-reset': ['Counter graph in OrangeEditor', 'Increment adds one, Decrement subtracts one, and Reset sets Counter to zero.'],
  'json-extraction': ['JSON extraction graph in OrangeEditor', 'The manual trigger runs the variable write. The JSON nodes supply the value through data wires.'],
  'power-with-count': ['Power graph with a loop in OrangeEditor', 'Count runs body four times. Each pass multiplies the variable by the base.'],
  'editor-counter-properties': ["OrangeEditor with Add one selected and its Properties open", "Select a card to see its settings in Properties. Add one is an Increment Counter bound to Counter."],
  'editor-counter-variables': ["OrangeEditor running the counter graph with Variables open", "After three presses of Increment, Variables shows Counter at 3, and Add one shows the old and new values it last wrote."],
  'power-loop-start': ["The start of the loop graph in OrangeEditor", "The trigger sets the variable to 1, then Count runs its body four times."],
  'http-with-failure-handling': ["HTTP request graph in OrangeEditor", "Every answer continues on Exit, where a Branch reads success. Failed runs only when no answer arrived."],
  'http-with-bounded-retry': ["HTTP retry graph in OrangeEditor", "A failed first request waits 250 ms and tries once more. A second failure is final."],
  'editor-http-connections': ["OrangeEditor with the Connections tab open", "The HTTP connection holds the base address. Each request chooses it and adds only a path."],
  'editor-file-trigger': ["OrangeEditor with On file change selected", "The trigger watches the incoming folder inside the granted folder. Its payload holds what changed."],
  'file-change-journal': ["File change journal graph in OrangeEditor", "Each change reads the path from the payload, adds a line break and appends it to changes.log."],
  'shared-score': ["Shared score graph in OrangeEditor", "Add a point adds one to the shared Score, and Reset sets it to zero. Every running graph that declares Score sees the change."],
  graph: 'Open the example graph',
  linkedPorts: 'Follow the ports',
};

export const kinds = {
  entry: 'Starts an activation when its trigger fires. Connect an execution output to the first step.',
  pure: 'Produces data for downstream inputs. A downstream read asks for the result.',
  volatile: 'Supplies a value that can change across evaluation epochs.',
  action: 'Runs when execution reaches it and follows one of its outgoing routes.',
  async: 'Starts work when execution reaches it and resumes the flow on an outcome.',
  iterator: 'Supplies items one at a time when the loop requests the next item.',
};

export const commonPorts = {
  in: 'Connect the execution wire that starts this step.',
  then: 'Continue along this execution route after the step completes.',
  failed: 'Follow this execution route when the operation faults.',
  cancelled: 'Follow this execution route when the work is cancelled.',
  body: 'Runs the loop body for the current iteration.',
  completed: 'Continues after the loop finishes.',
};

export const examples = {
  'counter-with-reset': ['Count button presses', 'Run the graph and fire Increment three times. Counter reads 3. Fire Decrement once to see 2, then Reset to return to 0.'],
  'set-on-trigger': ['Set a value on demand', 'Run the graph and fire On manual. Open Variables and confirm total is 7.'],
  'idle-watchdog': ['Restart an idle countdown', 'Fire Heartbeat twice within 750 ms. The next Listen activation replaces the earlier countdown. Idle becomes true after the latest countdown ends.'],
  'power-with-count': ['Compute a power using a loop', 'Run Calculate power. The example starts the result at 1, multiplies it by the base on each iteration, then publishes the final value.'],
  'running-total-with-reduce': ['Accumulate a running total', 'Open the graph and follow the running state from Reduce begin through Add to Reduce end. Inspect the final sum.'],
  'json-extraction': ['Read a field from JSON', 'Run the graph and fire its manual trigger. Follow the parsed document into JSON Read Text and inspect the variable it writes.'],
  'http-with-failure-handling': ['Route the result of an HTTP request', 'Choose a reachable test endpoint in the connection. Fire the graph and compare an HTTP success status, an HTTP error status and a connection failure.'],
  'http-with-bounded-retry': ["Retry an HTTP request once", "Choose a reachable test endpoint in the connection and fire Check endpoint. Stop the server to see the retry, then the final failure."],
  'file-change-journal': ["Keep a journal of file changes", "Point the folder connection at a folder with an incoming folder inside it, run the graph and add a file there. Its path appears in changes.log."],
  'shared-score': ["Share a score between graphs", "Run the graph and fire Add a point twice. Score reads 2 in Variables, and in every other running graph that declares it."],
  'quiet-mic-gate': ['Meter audio after a noise gate', 'Choose an input device and run the graph. Watch the gated level as the source becomes quiet. The example uses silent devices until you configure real ones.'],
};

export const notes = {
  'core/get-variable': 'Create a variable and select it in Properties. The Value output takes the variable’s type.',
  'org.orange.collections/enum-payload': 'Choose an enum and a variant that carries a payload. The enum and variant determine the port types.',
  'core/branch': 'Wire On manual into In and a Boolean set to true into Condition. Have True write matched to a text variable named Route, and False write skipped. Run the graph and press Fire. Route reads matched. Set the Boolean to false and fire again. Route reads skipped.',
  'core/bridge-in': 'Choose a Bridge Out channel and connect Result to a value input. The consuming node pulls the shared value.',
  'core/bridge-out': 'Connect a source to Value, name the channel, then choose it on one or more Bridge In cards.',
  'core/coalesce': 'Feed a missing optional number and set Fallback to 10. The result is 10. Supply an optional 4 and the result becomes 4.',
  'core/emit': 'Select a typed event to add its Payload input. Then choose the same event on a Listen card and fire Dispatch.',
  'core/on-event': 'Choose the event used by Dispatch. When that event is emitted, this card starts a fresh activation.',
  'org.orange.math/add': 'Set A to 2 and B to 3 using the same number type. A downstream read of Sum yields 5.',
  'org.orange.math/multiply': 'Set A to 3 and B to 4 using the same number type. A downstream read of Product yields 12.',
};
