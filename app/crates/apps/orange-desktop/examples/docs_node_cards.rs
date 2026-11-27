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

fn render_tutorial_graphs(
    host: &mut Host,
    gpu: &mut Offscreen,
    ctx: &egui::Context,
    destination: &std::path::Path,
    language: &str,
) -> Result<(), Box<dyn Error>> {
    let counter = include_str!("../../orange-cli/graphs/counter-with-reset.json");
    let json = include_str!("../../orange-cli/graphs/json-extraction.json");
    for (slug, document, dimensions) in [
        ("counter-with-reset", counter, [680, 850]),
        ("json-extraction", json, [1000, 520]),
    ] {
        let directory = destination.join(language).join("graphs").join(slug);
        fs::create_dir_all(&directory)?;
        let mut editor = Editor::new();
        editor.set_language(language);
        editor.open_untitled(host, document.to_owned());
        editor.canvas().fit();
        let [width, height] = dimensions;
        #[expect(clippy::cast_precision_loss, reason = "fixed small preview dimensions")]
        let bounds = Rect::from_min_size(Pos2::ZERO, egui::vec2(width as f32, height as f32));
        for (mode, theme) in SCHEMES {
            ctx.set_theme(theme);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    ..Default::default()
                },
                |ui| editor.canvas_panel(ui, host),
            );
            let rgba = gpu.render(ctx, output, dimensions)?;
            save_png(directory.join(format!("{mode}.png")), dimensions, &rgba)?;
            if slug == "counter-with-reset" {
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
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = PathBuf::from(args.next().ok_or("expected the preview output directory")?);
    let (mut host, _) = Host::with_services(
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
        render_tutorial_graphs(&mut host, &mut gpu, &ctx, &destination, language)?;
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
