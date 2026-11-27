//! `orange-componentize <core.wasm> <component.wasm>`: encodes a core module built with
//! wit-bindgen (which embeds its WIT world) as a validated component.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: orange-componentize <core.wasm> <component.wasm>");
        return ExitCode::from(2);
    };
    let result = std::fs::read(input)
        .map_err(|error| format!("read {input}: {error}"))
        .and_then(|module| {
            wit_component::ComponentEncoder::default()
                .validate(true)
                .module(&module)
                .and_then(|encoder| encoder.encode())
                .map_err(|error| format!("componentize: {error:#}"))
        })
        .and_then(|component| {
            std::fs::write(output, component).map_err(|error| format!("write {output}: {error}"))
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
