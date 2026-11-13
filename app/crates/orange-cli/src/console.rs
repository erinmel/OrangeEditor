//! The `run` command: printing what flows do and taking commands while they run.
use crate::*;

pub(crate) fn print(event: &HostEvent, names: &Names) {
    let flow = event.flow;
    match &event.kind {
        FlowEvent::State(state) => println!("[{flow}] state {state:?}"),
        FlowEvent::Node(step) => match &step.fault {
            None => println!(
                "[{flow}] {}: {}",
                names.node(&step.node),
                said(step.outcome)
            ),
            Some(fault) => println!(
                "[{flow}] {}: {}, fault {} from {}",
                names.node(&step.node),
                said(step.outcome),
                fault.fault.code,
                names.node(&fault.source)
            ),
        },
        FlowEvent::TraceLost(count) => println!("[{flow}] {count} steps not shown"),
        FlowEvent::Refused(reason) => println!("[{flow}] input refused: {reason:?}"),
        FlowEvent::Fault(error) => println!("[{flow}] error: {error}"),
        FlowEvent::Variables(changed) => {
            for (variable, value) in changed {
                match value {
                    Some(value) => {
                        println!("[{flow}] {} = {}", names.variable(variable), plain(value));
                    }
                    None => println!("[{flow}] {} is unset", names.variable(variable)),
                }
            }
        }
        FlowEvent::Ports(changed) => {
            for (node, port, value) in changed {
                println!("[{flow}] {} {port} = {}", names.node(node), plain(value));
            }
        }
        FlowEvent::Updated { .. } => println!("[{flow}] graph replaced"),
        FlowEvent::Finished => println!("[{flow}] finished"),
        _ => println!("[{flow}] {:?}", event.kind),
    }
}

/// Applies one line typed by the user. Returns `false` for `quit`.
/// Applies an edited graph file to its running flow.
pub(crate) fn reload(host: &mut Host, opened: &mut Opened) -> Result<(), String> {
    let (flow, path) = (opened.flow, opened.path.as_str());
    let text = text(path)?;
    let names = Names::of(host, &text);
    match host.update(flow, &text) {
        Ok(updated) if !updated.changed => println!("[{flow}] nothing to apply"),
        Ok(updated) => println!(
            "[{flow}] edit applied: {} variables kept, {} reset",
            updated.kept_variables, updated.reset_variables
        ),
        Err(error) => {
            report(path, &error, &names);
            println!("[{flow}] edit refused; the flow is unchanged");
            return Ok(());
        }
    }
    opened.names = names;
    Ok(())
}

/// The rest of a line after its first `skip` words, spaces inside it kept.
fn rest(line: &str, skip: usize) -> &str {
    let mut rest = line.trim_start();
    for _ in 0..skip {
        rest = rest
            .split_once(char::is_whitespace)
            .map_or("", |(_, after)| after.trim_start());
    }
    rest.trim_end()
}

pub(crate) fn command(host: &mut Host, flows: &mut [Opened], line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let at = words
        .get(1)
        .and_then(|word| word.parse::<usize>().ok())
        .and_then(|number| number.checked_sub(1))
        .filter(|at| *at < flows.len());
    let done = |result: Result<(), HostError>| result.map_err(|error| error.to_string());
    let opened = |at: Option<usize>| at.ok_or_else(|| "expected a flow number".to_owned());
    let outcome = match words.first().copied() {
        None => Ok(()),
        Some("quit") => return false,
        Some("pause") => opened(at).and_then(|at| done(host.pause(flows[at].flow))),
        Some("resume") => opened(at).and_then(|at| done(host.resume(flows[at].flow))),
        Some("reload") => opened(at).and_then(|at| reload(host, &mut flows[at])),
        Some("stop") => {
            opened(at).and_then(|at| done(host.stop(flows[at].flow, StopMode::Graceful)))
        }
        Some("kill") => {
            opened(at).and_then(|at| done(host.stop(flows[at].flow, StopMode::Immediate)))
        }
        Some("trigger") => opened(at).and_then(|at| {
            let flow = &flows[at];
            // A name may contain spaces: it is the rest of the line.
            let node = match rest(line, 2) {
                "" => {
                    let entries = host
                        .info(flow.flow)
                        .map(|info| info.manual_entries.clone())
                        .unwrap_or_default();
                    match entries[..] {
                        [(only, _)] => only,
                        [] => return Err("the graph has no manual entry".to_owned()),
                        _ => {
                            return Err("the graph has several manual entries; name one".to_owned());
                        }
                    }
                }
                name => flow.names.find_node(name)?,
            };
            done(host.trigger(flow.flow, node, None))
        }),
        Some("set") => opened(at).and_then(|at| {
            let flow = &flows[at];
            let name = words.get(2).ok_or("expected a variable")?;
            let (variable, ty) = flow.names.find_variable(name)?;
            let typed = rest(line, 3);
            if typed.is_empty() {
                return Err("expected a value".to_owned());
            }
            let value = host
                .value_from_json(&literal(&ty, typed)?)
                .map_err(|error| error.to_string())?;
            done(host.set_variable(flow.flow, variable, value))
        }),
        Some(other) => Err(format!("unknown command `{other}`")),
    };
    if let Err(message) = outcome {
        eprintln!("{message}");
    }
    true
}

pub(crate) fn run(
    host: &mut Host,
    events: &Receiver<HostEvent>,
    flows: &mut [Opened],
    duration: Option<Duration>,
) {
    let (lines, typed) = channel::<String>();
    // Standard input blocks, so it gets its own thread; it ends with the process.
    let _ = std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut running = flows.len();
    let mut stopping = false;
    while running > 0 {
        match events.recv_timeout(Duration::from_millis(20)) {
            Ok(event) => {
                match flows.iter().find(|opened| opened.flow == event.flow) {
                    Some(opened) => print(&event, &opened.names),
                    None => print(&event, &Names::default()),
                }
                if matches!(event.kind, FlowEvent::Finished) {
                    running -= 1;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let mut quit = duration.is_some_and(|limit| started.elapsed() >= limit);
        while let Ok(line) = typed.try_recv() {
            quit |= !command(host, flows, &line);
        }
        if quit && !stopping {
            stopping = true;
            for opened in flows.iter() {
                // A flow that already ended refuses the stop; that is the goal anyway.
                let _ = host.stop(opened.flow, StopMode::Graceful);
            }
        }
    }
    let dropped = host.events_dropped();
    if dropped > 0 {
        println!("{dropped} events were not shown");
    }
}
