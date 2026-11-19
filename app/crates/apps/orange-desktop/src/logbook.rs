//! Log message: a graph writes its own line to the editor's Log. The node only puts
//! the message on a bounded queue and goes on; the window takes the queue into the Log
//! between frames, so a graph that logs often never waits for the Log.
use orange_editor_ui::NoteLevel;
use orange_sdk::{Call, Definition, FaultText, KernelFault, ServiceNode, ServicePackage, Value};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

/// The package's ID.
const PACKAGE: &str = "org.orange.logging";
/// Most messages waiting for the Log; more are counted and dropped.
const MAX_WAITING: usize = 256;
const SEVERITY: &str = "org.orange.logging/severity@1";
const LEVEL: &str = "org.orange.logging/level@1";
const MAX_LOG_BYTES: usize = 16_384;
const LEVELS: [(&str, NoteLevel); 6] = [
    ("critical", NoteLevel::Critical),
    ("error", NoteLevel::Error),
    ("warn", NoteLevel::Warning),
    ("info", NoteLevel::Info),
    ("debug", NoteLevel::Debug),
    ("trace", NoteLevel::Trace),
];

#[derive(Debug, Default)]
struct Waiting {
    messages: VecDeque<(NoteLevel, String)>,
    dropped: u64,
}

/// The queue between Log message nodes and the editor's Log.
#[derive(Debug, Clone, Default)]
pub struct LogBook(Arc<Mutex<Waiting>>);

impl LogBook {
    /// The Logging package, whose node writes to this book.
    pub fn package(&self) -> Result<ServicePackage, orange_sdk::DefinitionError> {
        let definition = Definition::node(PACKAGE, "log-message", "Log message")
            .description("Writes to the Log. Add text arguments and insert them with {} or {0}, {1}, and so on. Use {{ and }} for literal braces.")
            .category("logging", "Logging")
            .keywords("log, print, message, format, debug, trace, write")
            .input("message", "core/string@1");
        let legacy = definition
            .clone()
            .input_default(
                "severity",
                SEVERITY,
                json!({"type": SEVERITY, "value": {"variant": "info"}}),
            )
            .build();
        let current = definition
            .version(2)
            .input_default(
                "level",
                LEVEL,
                json!({"type": LEVEL, "value": {"variant": "info"}}),
            )
            .variadic_input("argument", "core/string@1", 24)
            .build();
        ServicePackage::new(PACKAGE, "0.1.0")
            .about(
                "Logging",
                "Writes a graph's own messages to the editor's Log.",
            )
            .with_catalogs(crate::CATALOGS)
            .enumeration(
                "severity",
                &[("info", None), ("warning", None), ("error", None)],
            )
            .enumeration("level", &LEVELS.map(|(name, _)| (name, None)))
            .node(
                &legacy,
                Write {
                    waiting: Arc::clone(&self.0),
                    legacy: true,
                },
            )?
            .node(
                &current,
                Write {
                    waiting: Arc::clone(&self.0),
                    legacy: false,
                },
            )
    }

    /// Takes every waiting message, and how many were dropped since the last take.
    pub fn take(&self) -> (Vec<(NoteLevel, String)>, u64) {
        let mut waiting = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let dropped = std::mem::take(&mut waiting.dropped);
        (waiting.messages.drain(..).collect(), dropped)
    }
}

/// Puts a message on the queue, or counts it when the queue is full.
fn put(waiting: &Mutex<Waiting>, level: NoteLevel, text: &str) {
    let mut waiting = waiting.lock().unwrap_or_else(PoisonError::into_inner);
    if waiting.messages.len() >= MAX_WAITING {
        waiting.dropped += 1;
    } else {
        waiting.messages.push_back((
            level,
            text[..text.floor_char_boundary(MAX_LOG_BYTES)].to_owned(),
        ));
    }
}

#[derive(Debug)]
struct Write {
    waiting: Arc<Mutex<Waiting>>,
    legacy: bool,
}

impl ServiceNode for Write {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [
            Value::Str(message),
            Value::Enum { tag, payload: None },
            arguments @ ..,
        ] = call.inputs
        else {
            return Err(KernelFault {
                code: FaultText::Static("core/InputContract"),
                message_key: FaultText::Static("core/kernel-input-contract"),
                message: "Log message needs a message and a level",
            });
        };
        let level = if self.legacy {
            match tag {
                1 => NoteLevel::Warning,
                2 => NoteLevel::Error,
                _ => NoteLevel::Info,
            }
        } else {
            LEVELS
                .get(*tag as usize)
                .map_or(NoteLevel::Info, |(_, level)| *level)
        };
        if self.legacy {
            put(&self.waiting, level, message.as_str());
        } else {
            let text = format_message(message.as_str(), arguments);
            put(&self.waiting, level, &text);
        }
        Ok(Vec::new())
    }
}

/// Sequential or indexed text substitutions. Leave unknown placeholders visible.
fn format_message(template: &str, arguments: &[Value]) -> String {
    fn push(output: &mut String, text: &str) {
        let remaining = MAX_LOG_BYTES - output.len();
        output.push_str(&text[..text.floor_char_boundary(remaining)]);
    }

    let mut output = String::new();
    let mut rest = &template[..template.floor_char_boundary(MAX_LOG_BYTES)];
    let mut next = 0;
    while !rest.is_empty() && output.len() < MAX_LOG_BYTES {
        let Some(open) = rest.find(['{', '}']) else {
            push(&mut output, rest);
            break;
        };
        push(&mut output, &rest[..open]);
        rest = &rest[open..];
        if rest.starts_with("{{") || rest.starts_with("}}") {
            push(&mut output, &rest[..1]);
            rest = &rest[2..];
        } else if let Some(inner) = rest.strip_prefix('{')
            && let Some(end) = inner.find(|ch: char| !ch.is_ascii_digit())
            && inner.as_bytes()[end] == b'}'
        {
            let index = if end == 0 {
                let index = next;
                next += 1;
                Some(index)
            } else {
                inner[..end].parse().ok()
            };
            let placeholder = &rest[..end + 2];
            match index.and_then(|index| arguments.get(index)) {
                Some(Value::Str(value)) => push(&mut output, value.as_str()),
                _ => push(&mut output, placeholder),
            }
            rest = &rest[end + 2..];
        } else {
            push(&mut output, &rest[..1]);
            rest = &rest[1..];
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A graph's message reaches the book with its level, and a flood beyond the bound
    /// is counted instead of kept.
    #[test]
    fn messages_wait_in_order_and_a_flood_is_counted() {
        let book = LogBook::default();
        let call = |text: &str, level| put(&book.0, level, text);
        call("crossed", NoteLevel::Warning);
        let (messages, dropped) = book.take();
        assert_eq!(messages, [(NoteLevel::Warning, "crossed".to_owned())]);
        assert_eq!(dropped, 0);
        for _ in 0..MAX_WAITING + 5 {
            call("again", NoteLevel::Info);
        }
        let (messages, dropped) = book.take();
        assert_eq!(messages.len(), MAX_WAITING);
        assert_eq!(dropped, 5);
    }

    #[test]
    fn formatting_substitutes_positional_values_and_preserves_literal_braces() {
        let text = |s: &str| Value::Str(Arc::new(s.into()));
        let args = [text("Erin"), text("second")];
        assert_eq!(
            format_message("Hello {}, {1}, {{raw}}, {3}, {nonnumber}", &args),
            "Hello Erin, second, {raw}, {3}, {nonnumber}"
        );
    }

    /// A Log message node in a running graph puts its message on the book.
    #[test]
    fn a_running_graph_logs_through_the_node() {
        let book = LogBook::default();
        let (mut host, _) = orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![book.package().unwrap()],
        )
        .unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let start = host
            .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
            .unwrap();
        let log = host
            .add_node(&mut draft, "org.orange.logging/log-message", [300.0, 0.0])
            .unwrap();
        let text = host
            .add_node(&mut draft, "core/text", [0.0, 180.0])
            .unwrap();
        host.set_input(&mut draft, (text, "value"), Some("threshold"))
            .unwrap();
        let argument = host.add_port(&mut draft, log, "argument").unwrap();
        host.connect(&mut draft, (text, "result"), (log, &argument))
            .unwrap();
        let other = host.add_port(&mut draft, log, "argument").unwrap();
        host.set_input(&mut draft, (log, &other), Some("now"))
            .unwrap();
        host.set_input(&mut draft, (log, "message"), Some("crossed the {} {}"))
            .unwrap();
        host.set_input(
            &mut draft,
            (log, "level"),
            Some(&json!({"type": LEVEL, "value": {"variant": "debug"}}).to_string()),
        )
        .unwrap();
        host.connect(&mut draft, (start, "then"), (log, "in"))
            .unwrap();
        assert!(host.problems(&mut draft).unwrap().is_empty());
        let flow = host.open(&draft.text().unwrap()).unwrap();
        host.trigger(flow, start, None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let messages = loop {
            let (messages, _) = book.take();
            if !messages.is_empty() {
                break messages;
            }
            assert!(std::time::Instant::now() < deadline, "nothing was logged");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(
            messages,
            [(NoteLevel::Debug, "crossed the threshold now".to_owned())]
        );
        host.stop(flow, orange_host::StopMode::Graceful).unwrap();
        host.close(flow).unwrap();
    }
}
