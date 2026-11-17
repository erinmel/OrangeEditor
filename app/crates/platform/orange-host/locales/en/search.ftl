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
alias-org-orange-audio-input-level = microphone, mic, input, volume, loudness
alias-org-orange-audio-on-level = microphone, mic, sound, clap, loud
alias-org-orange-audio-play = sound, audio, speaker, music

purpose-core-on-manual = Starts when you press Run.
purpose-core-on-start = Starts once when the flow starts.
purpose-core-on-stop = Runs once when the flow stops.
purpose-core-on-timer = Starts again and again on a timer.
purpose-core-on-timeout = Starts once after a delay.
purpose-core-on-event = Starts when an event is emitted.
purpose-core-on-variable-changed = Starts when a variable changes.
purpose-core-on-watch = Starts when a watched value changes.
purpose-org-orange-audio-on-level = Starts when the microphone gets loud.
purpose-core-wait = Waits for a while, then goes on.
purpose-core-branch = Goes one way if true, the other if false.
purpose-core-whole-number = A whole number you type.
purpose-core-number = A decimal number you type.
purpose-core-true-or-false = True or false, as you set it.
purpose-core-text = A text you type.
