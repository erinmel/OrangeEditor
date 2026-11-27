//! Render the editor's read-only node cards for the static documentation.
use eframe::egui::{self, Color32, Pos2, Rect};
use eframe::egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};
use orange_editor_ui::{Editor, apply_theme, canvas, i18n, icons};
use orange_host::{Host, HostOptions};
use std::{error::Error, fs, io::BufWriter, path::PathBuf, sync::mpsc};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
}

impl Offscreen {
    async fn new() -> Result<Self, Box<dyn Error>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;
        let renderer = Renderer::new(&device, FORMAT, RendererOptions::default());
        Ok(Self {
            device,
            queue,
            renderer,
        })
    }

    fn render(
        &mut self,
        ctx: &egui::Context,
        mut output: egui::FullOutput,
        [width, height]: [u32; 2],
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        let screen = ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: output.pixels_per_point,
        };
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                self.renderer
                    .update_texture(&self.device, &self.queue, id, &delta);
            }
        }
        let primitives = ctx.tessellate(output.shapes, screen.pixels_per_point);
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("docs-node-card"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let preparation = self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &primitives,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &primitives, &screen);
        }

        let row = width * 4;
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let stride = row.div_ceil(alignment) * alignment;
        let pixels = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("docs-node-card-pixels"),
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &pixels,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        let submitted = self
            .queue
            .submit(preparation.into_iter().chain([encoder.finish()]));
        for id in output.textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }
        let slice = pixels.slice(..);
        let (send, receive) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(std::time::Duration::from_secs(15)),
        })?;
        receive.recv()??;
        let rgba = slice
            .get_mapped_range()?
            .chunks_exact(stride as usize)
            .flat_map(|line| line[..row as usize].iter().copied())
            .collect();
        Ok(rgba)
    }
}

fn css_colors(ctx: &egui::Context, theme: egui::Theme) -> String {
    let style = ctx.style_of(theme);
    let visuals = &style.visuals;
    let colors = [
        ("bg", visuals.extreme_bg_color),
        ("panel", visuals.window_fill),
        ("text", visuals.strong_text_color()),
        ("muted", visuals.weak_text_color()),
        ("line", visuals.window_stroke.color),
        (
            "control-border",
            visuals.widgets.noninteractive.bg_stroke.color,
        ),
        ("soft", visuals.faint_bg_color),
        ("accent", visuals.selection.stroke.color),
        ("primary-fill", visuals.selection.bg_fill),
    ];
    use std::fmt::Write as _;
    colors.iter().fold(String::new(), |mut css, (name, color)| {
        let [r, g, b, _] = color.to_array();
        let _ = writeln!(css, "  --{name}: #{r:02x}{g:02x}{b:02x};");
        css
    })
}

fn save_png(path: PathBuf, [width, height]: [u32; 2], rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut png = png::Encoder::new(BufWriter::new(fs::File::create(path)?), width, height);
    png.set_color(png::ColorType::Rgba);
    png.write_header()?.write_image_data(rgba)?;
    Ok(())
}

const SCHEMES: [(&str, egui::Theme); 2] =
    [("light", egui::Theme::Light), ("dark", egui::Theme::Dark)];

/// What a documentation picture shows of the editor.
enum View {
    /// The canvas alone, fitted to the graph.
    Canvas,
    /// The whole editor, as a person sees it: optionally with a node selected so its
    /// properties show, buttons fired while the graph runs, and then labels pressed in
    /// order, named by their catalogue message (a tab, or Advanced).
    Editor {
        press: &'static [&'static str],
        select: Option<&'static str>,
        fire: &'static [(&'static str, usize)],
    },
}

/// A picture for the guides: its folder name, the graph, the names of the nodes to keep
/// (all when empty), what it shows and its size in pixels.
struct Shot {
    slug: &'static str,
    graph: &'static str,
    keep: &'static [&'static str],
    view: View,
    size: [u32; 2],
}

const COUNTER: &str = include_str!("../../orange-cli/graphs/counter-with-reset.json");
const JSON: &str = include_str!("../../orange-cli/graphs/json-extraction.json");
const POWER: &str = include_str!("../../orange-cli/graphs/power-with-count.json");
const HTTP: &str = include_str!("../../orange-cli/graphs/http-with-failure-handling.json");
const RETRY: &str = include_str!("../../orange-cli/graphs/http-with-bounded-retry.json");
const FILES: &str = include_str!("../../orange-cli/graphs/file-change-journal.json");
const SHARED: &str = include_str!("../../orange-cli/graphs/shared-score.json");
const IDE: [u32; 2] = [1440, 860];

const SHOTS: &[Shot] = &[
    Shot {
        slug: "counter-with-reset",
        graph: COUNTER,
        keep: &[],
        view: View::Canvas,
        size: [680, 850],
    },
    Shot {
        slug: "json-extraction",
        graph: JSON,
        keep: &[],
        view: View::Canvas,
        size: [1000, 520],
    },
    Shot {
        slug: "power-with-count",
        graph: POWER,
        keep: &[],
        view: View::Canvas,
        size: [1280, 680],
    },
    Shot {
        slug: "power-loop-start",
        graph: POWER,
        keep: &[
            "Calculate power",
            "Reset accumulator to 1",
            "Count exponent iterations",
            "Exponent",
        ],
        view: View::Canvas,
        size: [1000, 420],
    },
    Shot {
        slug: "http-with-failure-handling",
        graph: HTTP,
        keep: &[],
        view: View::Canvas,
        size: [1280, 640],
    },
    Shot {
        slug: "http-with-bounded-retry",
        graph: RETRY,
        keep: &[],
        view: View::Canvas,
        size: [1280, 640],
    },
    Shot {
        slug: "file-change-journal",
        graph: FILES,
        keep: &[],
        view: View::Canvas,
        size: [1000, 520],
    },
    Shot {
        slug: "shared-score",
        graph: SHARED,
        keep: &[],
        view: View::Canvas,
        size: [680, 560],
    },
    Shot {
        slug: "editor-counter-properties",
        graph: COUNTER,
        keep: &[],
        view: View::Editor {
            press: &[],
            select: Some("Add one"),
            fire: &[],
        },
        size: IDE,
    },
    Shot {
        slug: "editor-counter-variables",
        graph: COUNTER,
        keep: &[],
        view: View::Editor {
            press: &["variables"],
            select: None,
            fire: &[("Increment", 3)],
        },
        size: IDE,
    },
    Shot {
        slug: "editor-http-connections",
        graph: HTTP,
        keep: &[],
        view: View::Editor {
            press: &["connections"],
            select: None,
            fire: &[],
        },
        size: IDE,
    },
    Shot {
        slug: "editor-file-trigger",
        graph: FILES,
        keep: &[],
        view: View::Editor {
            press: &["connections"],
            select: Some("On file change"),
            fire: &[],
        },
        size: IDE,
    },
];

/// The graph with only the named nodes and the wires between them.
fn only_nodes(graph: &str, keep: &[&str]) -> Result<String, Box<dyn Error>> {
    if keep.is_empty() {
        return Ok(graph.to_owned());
    }
    let mut document: serde_json::Value = serde_json::from_str(graph)?;
    let nodes = document["nodes"]
        .as_array_mut()
        .ok_or("a graph has nodes")?;
    nodes.retain(|node| keep.iter().any(|name| node["name"] == *name));
    let ids: Vec<serde_json::Value> = nodes.iter().map(|node| node["id"].clone()).collect();
    if ids.len() != keep.len() {
        return Err(format!("a documentation picture names missing nodes: {keep:?}").into());
    }
    if let Some(edges) = document["edges"].as_array_mut() {
        edges.retain(|edge| {
            ids.contains(&edge["from"]["node"]) && ids.contains(&edge["to"]["node"])
        });
    }
    Ok(document.to_string())
}

/// The ID of the node a graph names so.
fn node_named(graph: &str, name: &str) -> Result<orange_host::NodeId, Box<dyn Error>> {
    let document: serde_json::Value = serde_json::from_str(graph)?;
    let id = document["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["name"] == name))
        .and_then(|node| node["id"].as_str())
        .ok_or_else(|| format!("no node named {name}"))?;
    Ok(id.parse()?)
}

/// Where a label is drawn, to press it like a person would.
fn label_at(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
}

/// Every label a frame drew, to explain a missing one.
fn all_labels(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

fn render_tutorial_graphs(
    host: &mut Host,
    events: &orange_host::Events,
    gpu: &mut Offscreen,
    ctx: &egui::Context,
    destination: &std::path::Path,
    language: &str,
) -> Result<(), Box<dyn Error>> {
    for shot in SHOTS {
        let directory = destination.join(language).join("graphs").join(shot.slug);
        fs::create_dir_all(&directory)?;
        let mut editor = Editor::new();
        editor.set_language(language);
        editor.open_untitled(host, only_nodes(shot.graph, shot.keep)?);
        let [width, height] = shot.size;
        #[expect(clippy::cast_precision_loss, reason = "fixed small preview dimensions")]
        let bounds = Rect::from_min_size(Pos2::ZERO, egui::vec2(width as f32, height as f32));
        let frame = |editor: &mut Editor, host: &mut Host, input: Vec<egui::Event>| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    events: input,
                    ..Default::default()
                },
                |ui| match shot.view {
                    View::Canvas => editor.canvas_panel(ui, host),
                    View::Editor { .. } => editor.ui(ui, host),
                },
            )
        };
        if let View::Editor {
            press,
            select,
            fire,
        } = shot.view
        {
            if let Some(name) = select {
                editor.canvas().select(Some(node_named(shot.graph, name)?));
            }
            if !fire.is_empty() {
                editor.run(host);
                for &(name, times) in fire {
                    let node = node_named(shot.graph, name)?;
                    for _ in 0..times {
                        editor.trigger(host, node);
                    }
                }
                for _ in 0..40 {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    editor.poll_events(events);
                }
            }
            // Lay the panels out, then press the tab where the editor drew it.
            let output = frame(&mut editor, host, Vec::new());
            gpu.render(ctx, output, shot.size)?;
            for &label in press {
                // A narrow panel keeps some tabs behind its "…" menu: open it first.
                let mut targets = vec![i18n::t(label)];
                for _ in 0..2 {
                    let output = frame(&mut editor, host, Vec::new());
                    let found = label_at(&output, targets[targets.len() - 1]);
                    let more = label_at(&output, "…");
                    let texts = all_labels(&output);
                    gpu.render(ctx, output, shot.size)?;
                    let at = match (found, more) {
                        (Some(at), _) => at,
                        (None, Some(more)) if targets.len() == 1 => {
                            targets.insert(0, "…");
                            more
                        }
                        _ => return Err(format!("no {label} among {texts:?}").into()),
                    };
                    for pressed in [true, false] {
                        let click = egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        };
                        let output = frame(
                            &mut editor,
                            host,
                            vec![egui::Event::PointerMoved(at), click],
                        );
                        gpu.render(ctx, output, shot.size)?;
                    }
                    if found.is_some() {
                        break;
                    }
                }
            }
            editor.canvas().fit();
        } else {
            editor.canvas().fit();
        }
        for (mode, theme) in SCHEMES {
            ctx.set_theme(theme);
            // A frame to settle the layout and theme, then the picture.
            let output = frame(&mut editor, host, Vec::new());
            gpu.render(ctx, output, shot.size)?;
            let output = frame(&mut editor, host, Vec::new());
            let rgba = gpu.render(ctx, output, shot.size)?;
            save_png(directory.join(format!("{mode}.png")), shot.size, &rgba)?;
            if shot.slug == "counter-with-reset" {
                // The first row of the checked-in graph is the tutorial's first step.
                let first_row = destination
                    .join(language)
                    .join("graphs")
                    .join("counter-first-step");
                fs::create_dir_all(&first_row)?;
                let height = 280;
                let bytes = (width * height * 4) as usize;
                save_png(
                    first_row.join(format!("{mode}.png")),
                    [width, height],
                    &rgba[..bytes],
                )?;
            }
        }
        if editor.is_running() {
            editor.stop(host);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = PathBuf::from(args.next().ok_or("expected the preview output directory")?);
    let (mut host, events) = Host::with_services(
        HostOptions::default(),
        vec![
            orange_net::package(),
            orange_files::package(),
            orange_audio::package(),
        ],
    )
    .map_err(|error| error.to_string())?;
    let mut gpu = futures_lite::future::block_on(Offscreen::new())?;
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    icons::install(&ctx);
    fs::create_dir_all(&destination)?;
    let light = css_colors(&ctx, egui::Theme::Light);
    let dark = css_colors(&ctx, egui::Theme::Dark);
    fs::write(
        destination.join("editor-theme.css"),
        format!(
            ":root:not([data-theme]), :root[data-theme=\"light\"] {{\n{light}}}\n\
             :root[data-theme=\"dark\"] {{\n{dark}}}\n\
             @media (prefers-color-scheme: dark) {{\n:root:not([data-theme]) {{\n{dark}}}\n}}\n"
        ),
    )?;
    let padding = egui::Vec2::splat(12.0);

    for language in ["en", "es"] {
        render_tutorial_graphs(&mut host, &events, &mut gpu, &ctx, &destination, language)?;
        i18n::set_language(language);
        i18n::set_port_names(host.port_names(language));
        for info in host.nodes() {
            let definition = &info.definition.definition;
            let key = definition.id.as_str();
            let node = canvas::documentation_card_view(&host, key)
                .ok_or_else(|| format!("node palette has no card for {key}"))?;
            let label = host.label_in(&info, language);
            let size = canvas::node_size(&node) + 2.0 * padding;
            let dimensions = [size.x, size.y].map(|axis| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "bounded positive node size"
                )]
                let pixels = axis.ceil() as u32;
                pixels
            });
            let directory = destination
                .join(language)
                .join(key)
                .join(definition.version.to_string());
            fs::create_dir_all(&directory)?;
            for (mode, theme) in SCHEMES {
                ctx.set_theme(theme);
                let bounds = Rect::from_min_size(Pos2::ZERO, size);
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(bounds),
                        ..Default::default()
                    },
                    |ui| {
                        let background: Color32 = ui.visuals().extreme_bg_color;
                        ui.painter().rect_filled(bounds, 0.0, background);
                        canvas::documentation_card(ui, &node, &label, Pos2::ZERO + padding);
                    },
                );
                let rgba = gpu.render(&ctx, output, dimensions)?;
                save_png(directory.join(format!("{mode}.png")), dimensions, &rgba)?;
            }
        }
    }
    Ok(())
}
