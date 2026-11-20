# Words the node search finds nodes by, in English (docs/13, docs/22 §4).
#
# concept-<id>          words for a purpose that groups nodes (comma separated)
# concept-<id>-purpose  one line on what nodes of that purpose do
# category-<slug>       more words for a palette category
# alias-<node key>      more words for one node; the key is its ID with "/" and "." as "-"
# purpose-<node key>    one line on what one node does
#
# IDs never change with the language; a locale only adds words.

language-name = English

concept-trigger = trigger, when, start, event, on, entry, begin, run
concept-trigger-purpose = Starts the flow when something happens.
concept-math = math, maths, arithmetic, calculate, number, sum
concept-math-purpose = Calculates with numbers.
concept-compare = compare, comparison, equal, greater, less, same, check
concept-compare-purpose = Compares two values.
concept-logic = logic, if, condition, boolean, branch, choose
concept-logic-purpose = Chooses a way on a condition.
concept-text = text, string, words, characters, letters
concept-text-purpose = Works with text.
concept-time = time, clock, timer, delay, wait, interval, schedule
concept-time-purpose = Waits for or reads the time.
concept-audio = audio, sound, music, microphone, speaker, volume
concept-audio-purpose = Plays or listens to sound.
concept-network = network, internet, web, http, request, mqtt, websocket, online
concept-network-purpose = Talks to other machines.
concept-file = file, folder, disk, path, directory, save, load
concept-file-purpose = Reads or writes files and folders.
concept-variable = variable, state, store, remember, memory, value
concept-variable-purpose = Reads or keeps a value between runs.
concept-loop = loop, repeat, each, iterate, cycle, times
concept-loop-purpose = Repeats work.
concept-list = list, array, collection, map, dictionary, record
concept-list-purpose = Works with lists, maps and records.
concept-data = json, data, parse, field
concept-data-purpose = Reads values out of JSON.

alias-core-whole-number = integer, whole number, i64, u64, count, number
alias-core-number = number, decimal, real, f64, f32, double
alias-core-true-or-false = boolean, true or false, flag, switch
alias-core-text = string, str, words
alias-core-value = constant, literal
alias-core-wait = delay, sleep, pause
alias-core-set-variable = assign, store, write
alias-core-get-variable = read, load
alias-core-branch = if, else, condition
alias-core-count = for loop, counted loop, repeat n times, repeat times
purpose-core-count = Runs Body once per count, with an index starting at zero. Completed runs when the loop ends.
alias-org-orange-audio-play = sound, audio, speaker, music

purpose-core-on-manual = Starts when you press Run.
purpose-core-on-start = Starts once when the flow starts.
purpose-core-on-stop = Runs once when the flow stops.
purpose-core-on-timer = Starts again and again on a timer.
purpose-core-on-timeout = Starts once after a delay.
purpose-core-on-event = Starts when an event is emitted.
purpose-core-on-variable-changed = Starts when a variable changes.
purpose-core-on-watch = Starts when a watched value changes, rises to true, or crosses a threshold or range.
alias-core-on-watch = on watch, threshold, condition, rising, range
purpose-core-wait = Waits for a while, then goes on.
purpose-core-branch = Goes one way if true, the other if false.
purpose-core-whole-number = A whole number you type.
purpose-core-number = A decimal number you type.
purpose-core-true-or-false = True or false, as you set it.
purpose-core-text = A text you type.

# Collections and JSON, one line per node.
purpose-org-orange-collections-list-length = Counts the items of a list.
purpose-org-orange-collections-list-get = Gives the item at an index of a list, or nothing past its end.
purpose-org-orange-collections-list-slice = Gives part of a list: a count of items from a start index; fails past the end.
purpose-org-orange-collections-list-concat = Joins two lists, the items of a then those of b.
purpose-org-orange-collections-list-append = Gives the list with one more item at its end.
purpose-org-orange-collections-list-contains = Tells whether a list holds a value.
purpose-org-orange-collections-list-index-of = Gives where a value first appears in a list, or nothing.
purpose-org-orange-collections-list-reverse = Gives the list in the opposite order.
purpose-org-orange-collections-map-length = Counts the entries of a map.
purpose-org-orange-collections-map-get = Gives the value stored under a key, or nothing.
purpose-org-orange-collections-map-contains-key = Tells whether a map has a key.
purpose-org-orange-collections-map-insert = Gives the map with a value stored under a key, replacing any there.
purpose-org-orange-collections-map-remove = Gives the map without a key and its value.
purpose-org-orange-collections-map-keys = Lists the keys of a map.
purpose-org-orange-collections-map-values = Lists the values of a map.
purpose-org-orange-collections-option-is-some = Tells whether an optional value holds something.
purpose-org-orange-collections-make-record = Builds a record of one of the graph's types from its fields.
purpose-org-orange-collections-get-field = Reads one field of a record.
purpose-org-orange-collections-set-field = Gives the record with one field changed.
purpose-org-orange-collections-make-enum = Picks one choice of an enum, with its payload when it has one.
purpose-org-orange-collections-enum-payload = Reads the payload a choice of an enum carries. Needs an enum with a choice that carries one.
purpose-org-orange-json-parse = Reads JSON text into a JSON value; fails on text that is not JSON.
purpose-org-orange-json-kind = Says what a JSON value is: object, array, string, number, boolean or null.
purpose-org-orange-json-is-null = Tells whether a JSON value is null.
purpose-org-orange-json-get = Gives the field of a JSON object by name, or nothing when it has none; fails on a value that is not an object.
purpose-org-orange-json-get-path = Reads nested JSON fields and array items using a path like items[0].name or headers["Content-Type"].
purpose-org-orange-json-at = Gives the item of a JSON array at an index, or nothing past its end; fails on a value that is not an array.
purpose-org-orange-json-length = Counts the items of a JSON array or the fields of an object; fails on any other kind.
purpose-org-orange-json-as-bool = Gives a JSON boolean as true or false, or nothing for any other kind.
purpose-org-orange-json-as-string = Gives a JSON string as text, or nothing for any other kind.
purpose-org-orange-json-as-i64 = Gives a JSON number as a whole number, or nothing for another kind; fails on a fraction or one too large.
purpose-org-orange-json-as-u64 = Gives a JSON number as a whole number of zero or more, or nothing for another kind; fails on a fraction, a negative or one too large.
purpose-org-orange-json-as-f64 = Gives a JSON number as a decimal, or nothing for any other kind.
