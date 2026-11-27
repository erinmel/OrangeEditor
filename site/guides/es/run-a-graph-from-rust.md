# Ejecuta un grafo desde Rust

En este tutorial escribes un pequeño programa en Rust que abre el grafo del contador,
pulsa su botón **Increment** tres veces e imprime **Counter**. Conoces el
[`Host`](@shared/api/orange_host/struct.Host.html), que toda aplicación usa para
comprobar y ejecutar grafos, y el canal de eventos que informa de lo que hace un grafo
en ejecución. No hace falta ninguna ventana. Lleva unos quince minutos.

Necesitas Rust 1.97.1 o posterior y un clon del repositorio de OrangeEditor. Ayuda haber
hecho antes [Cuenta las pulsaciones de un botón](../count-button-presses/), pero el
grafo del contador también está en el repositorio.

El programa terminado es
[`examples/run_counter.rs`](@source/app/crates/platform/orange-host/examples/run_counter.rs).
Desde la raíz del repositorio, `cargo run -p orange-host --example run_counter --locked`
lo ejecuta.

<h2 id="create-the-project">Crea el proyecto</h2>

Crea un proyecto junto a tu clon de OrangeEditor:

```sh
cargo new counter-runner
cd counter-runner
```

Añade `orange-host` desde el clon, y `serde_json` para leer nombres del archivo del
grafo, en `Cargo.toml`:

```toml
[dependencies]
orange-host = { path = "../OrangeEditor/app/crates/platform/orange-host" }
serde_json = "1"
```

Copia el grafo que guardaste en el tutorial del contador como `counter.json` en la
carpeta del proyecto, o copia `app/crates/apps/orange-cli/graphs/counter-with-reset.json`
desde el clon.

<h2 id="check-the-graph">Comprueba el grafo</h2>

Un host contiene el catálogo de nodos y ejecuta grafos. Basta con uno por aplicación.
[`Host::check`](@shared/api/orange_host/struct.Host.html#method.check) informa de lo
que mostraría el panel **Problemas** del editor, sin ejecutar nada. Sustituye
`src/main.rs` por:

```rust
use orange_host::{FlowEvent, Host, HostOptions, NodeId, Severity, StopMode, Value, VariableId};
use std::error::Error;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let text = std::fs::read_to_string("counter.json")?;
    let (mut host, events) = Host::new(HostOptions::default())?;

    let mut blocked = false;
    for problem in host.check(&text)? {
        eprintln!("{:?}: {}", problem.severity, problem.message);
        blocked |= problem.severity == Severity::Error;
    }
    if blocked {
        return Err("the graph has errors; open it in the editor to fix them".into());
    }
    Ok(())
}
```

Ejecuta `cargo run`. No imprime nada y termina: el grafo no tiene problemas. Cambia
`"core/i64@1"` en el `type` de la variable por `"core/i64@9"` y vuelve a ejecutarlo para
ver un error; después deja el valor como estaba.

<h2 id="find-the-button-and-the-variable">Busca el botón y la variable</h2>

Un grafo se refiere a nodos y variables por su ID, y las personas les ponen nombre.
Añade esta función encima de `main` para buscar un ID por su nombre en el archivo del
grafo:

```rust
/// The ID of the node or variable a graph file gives this name.
fn id_named<T: std::str::FromStr>(graph: &serde_json::Value, list: &str, name: &str) -> Option<T> {
    graph[list]
        .as_array()?
        .iter()
        .find(|item| item["name"] == name)?["id"]
        .as_str()?
        .parse()
        .ok()
}
```

Después, en `main`, antes de `Ok(())`:

```rust
    let graph: serde_json::Value = serde_json::from_str(&text)?;
    let increment: NodeId = id_named(&graph, "nodes", "Increment").ok_or("no Increment node")?;
    let counter: VariableId =
        id_named(&graph, "variables", "Counter").ok_or("no Counter variable")?;
```

<h2 id="press-the-button">Pulsa el botón</h2>

[`Host::open`](@shared/api/orange_host/struct.Host.html#method.open) inicia el grafo
como un flujo en su propio hilo y vuelve enseguida.
[`Host::trigger`](@shared/api/orange_host/struct.Host.html#method.trigger) hace lo
mismo que **Disparar** en el editor. Cada llamada pone una activación en la cola.

```rust
    let flow = host.open(&text)?;
    host.observe(flow, true)?;
    for _ in 0..3 {
        host.trigger(flow, increment, None)?;
    }
```

`observe` pide al flujo que informe de sus variables. Sin él, el flujo se ejecuta igual,
pero en silencio.

<h2 id="read-what-happens">Lee lo que pasa</h2>

Todo lo que hace cada flujo llega por el canal `events` que devolvió `Host::new`. Espera
a que **Counter** llegue a 3, detén el flujo y espera a que termine:

```rust
    let mut stopping = false;
    loop {
        let event = events.recv_timeout(Duration::from_secs(10))?;
        match event.kind {
            FlowEvent::Variables(changed) => {
                for (variable, value) in changed {
                    if variable != counter {
                        continue;
                    }
                    println!("Counter = {value:?}");
                    if value == Some(Value::I64(3)) && !stopping {
                        stopping = true;
                        host.stop(flow, StopMode::Graceful)?;
                    }
                }
            }
            FlowEvent::Fault(error) => return Err(format!("{error:?}").into()),
            FlowEvent::Finished => break,
            _ => {}
        }
    }
    host.close(flow)?;
```

Ejecuta `cargo run`. La última línea que imprime es:

```text
Counter = Some(I64(3))
```

Puede que antes veas `Some(I64(1))` o `Some(I64(2))`. Cada informe contiene el último
valor de cada variable que cambió desde el informe anterior, así que varios cambios
rápidos pueden llegar juntos.

<h2 id="what-you-built">Qué construiste</h2>

- **Un host, muchos flujos.** `Host::open` puede ejecutar varios grafos a la vez, cada
  uno en su hilo, y todos informan por el mismo canal. `event.flow` indica cuál.
- **El host es el motor del editor.** El editor de escritorio usa estas mismas
  llamadas; dibuja los eventos en lugar de imprimirlos.
- **Los archivos de grafo son datos.** Tu programa lee nombres del JSON, pero es el host
  quien lo comprueba y ejecuta; tú nunca ejecutas nada del archivo.

Después, lleva el editor a tu propia aplicación con
[Integra y personaliza el editor egui](../egui/), o consulta la
[API de `orange-host`](@shared/api/orange_host/) e [Integra el motor](../embedding/)
para servicios, paquetes y variables compartidas.
