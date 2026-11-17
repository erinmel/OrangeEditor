//! An example native plugin library.
//!
//! It adds one node, `org.example.shout/shout`, which returns its text in upper case
//! followed by an exclamation mark. Build it with `cargo build -p orange-plugin-example`
//! and put the resulting library in the directory the application loads plugins from
//! (`orange --plugins <dir>`).
//!
//! A plugin of your own is a crate like this one: `crate-type = ["cdylib"]`, a
//! dependency on `orange-sdk`, a function returning a `ServicePackage`, and one
//! `export_plugin!` line. Build it with the same compiler and lockfile as the
//! application that loads it.
use orange_sdk::{Call, Definition, FaultText, KernelFault, ServiceNode, ServicePackage, Value};
use std::sync::Arc;

/// Package namespace of this plugin.
pub const PACKAGE: &str = "org.example.shout";

const NOT_TEXT: KernelFault = KernelFault {
    code: FaultText::Static("org.example.shout/Input"),
    message_key: FaultText::Static("org.example.shout/input-invalid"),
    message: "The input is not text.",
};

#[derive(Debug)]
struct Shout;
impl ServiceNode for Shout {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [Value::Str(text)] = call.inputs else {
            return Err(NOT_TEXT);
        };
        Ok(vec![Value::Str(Arc::new(format!(
            "{}!",
            text.to_uppercase()
        )))])
    }
}

fn shout_definition() -> String {
    Definition::node(PACKAGE, "shout", "Shout")
        .category("text", "Example")
        .input("text", "core/string@1")
        .output("loud", "core/string@1")
        .build()
}

fn package() -> ServicePackage {
    let package = ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .about("Shout", "An example plugin: one node that shouts its text.")
        .node(&shout_definition(), Shout);
    match package {
        Ok(package) => package,
        // The definition is a constant of this crate, checked by the loader's tests.
        Err(error) => unreachable!("built-in definition is valid: {error}"),
    }
}

orange_sdk::export_plugin!(package);
