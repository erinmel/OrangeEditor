//! The `run` command: printing what flows do and taking commands while they run.
use crate::*;

pub(crate) fn print(event: &HostEvent) {
    let flow = event.flow;
    match &event.kind {
        FlowEvent::State(state) => println!("[{flow}] state {state:?}"),
        FlowEvent::Node(step) => match &step.fault {
            None => println!("[{flow}] node {} {:?}", step.node, step.outcome),
            Some(fault) => println!(
                "[{flow}] node {} {:?} fault {} from {}",
                step.node, step.outcome, fault.fault.code, fault.source
            ),
        },
        FlowEvent::TraceLost(count) => println!("[{flow}] {count} steps not shown"),
        FlowEvent::Refused(reason) => println!("[{flow}] input refused: {reason:?}"),
        FlowEvent::Fault(error) => println!("[{flow}] error: {error}"),
        FlowEvent::Variables(changed) => {
            for (variable, value) in changed {
                match value {
                    Some(value) => println!("[{flow}] variable {variable} = {value:?}"),
                    None => println!("[{flow}] variable {variable} is unset"),
                }
            }
        }
        FlowEvent::Ports(changed) => {
            for (node, port, value) in changed {
                println!("[{flow}] wire {node} {port} = {value:?}");
            }
        }
        FlowEvent::Updated { .. } => println!("[{flow}] graph replaced"),
        FlowEvent::Finished => println!("[{flow}] finished"),
        _ => println!("[{flow}] {:?}", event.kind),
    }
}

/// Applies one line typed by the user. Returns `false` for `quit`.
/// Applies an edited graph file to its running flow.
pub(crate) fn reload(host: &mut Host, flow: FlowId, path: &str) -> Result<(), String> {
    let text = String::from_utf8(read(path)?).map_err(|_| format!("{path}: not UTF-8 text"))?;
    match host.update(flow, &text) {
        Ok(updated) if !updated.changed => println!("[{flow}] nothing to apply"),
        Ok(updated) => println!(
            "[{flow}] edit applied: {} variables kept, {} reset",
            updated.kept_variables, updated.reset_variables
        ),
        Err(error) => {
            report(path, &error);
            println!("[{flow}] edit refused; the flow is unchanged");
        }
    }
    Ok(())
}

pub(crate) fn command(host: &mut Host, flows: &[(FlowId, String)], line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let flow = |index: usize| -> Result<(FlowId, &str), String> {
        let number: usize = words
            .get(index)
            .and_then(|word| word.parse().ok())
            .ok_or("expected a flow number")?;
        number
            .checked_sub(1)
            .and_then(|at| flows.get(at))
            .map(|(flow, path)| (*flow, path.as_str()))
            .ok_or_else(|| "no such flow".to_owned())
    };
    let done = |result: Result<(), HostError>| result.map_err(|error| error.to_string());
    let outcome = match words.first().copied() {
        None => Ok(()),
        Some("quit") => return false,
        Some("pause") => flow(1).and_then(|(flow, _)| done(host.pause(flow))),
        Some("resume") => flow(1).and_then(|(flow, _)| done(host.resume(flow))),
        Some("reload") => flow(1).and_then(|(flow, path)| reload(host, flow, path)),
        Some("stop") => flow(1).and_then(|(flow, _)| done(host.stop(flow, StopMode::Graceful))),
        Some("kill") => flow(1).and_then(|(flow, _)| done(host.stop(flow, StopMode::Immediate))),
        Some("trigger") => flow(1).and_then(|(flow, _)| {
            let node: NodeId = words
                .get(2)
                .and_then(|word| word.parse().ok())
                .ok_or("expected a node id")?;
            done(host.trigger(flow, node, None))
        }),
        Some("set") => flow(1).and_then(|(flow, _)| {
            let variable = words
                .get(2)
                .and_then(|word| word.parse().ok())
                .ok_or("expected a variable id")?;
            // The literal is the rest of the line, spaces included.
            let literal = line
                .splitn(4, char::is_whitespace)
                .nth(3)
                .ok_or(r#"expected a literal such as {"type":"core/i64@1","value":"5"}"#)?;
            let value = host
                .value_from_json(literal.trim())
                .map_err(|error| error.to_string())?;
            done(host.set_variable(flow, variable, value))
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
    flows: &[(FlowId, String)],
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
                print(&event);
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
            for (flow, _) in flows {
                // A flow that already ended refuses the stop; that is the goal anyway.
                let _ = host.stop(*flow, StopMode::Graceful);
            }
        }
    }
    let dropped = host.events_dropped();
    if dropped > 0 {
        println!("{dropped} events were not shown");
    }
}
