use std::{f32::consts::TAU, time::Duration};

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use orange_ui_controls::{
    Divider, MeterOrientation, Surface, UiTokens,
    audio_meter::{
        ChannelLevel, ChannelReading, MeterStyle, PeakBallistics, RmsMeter, amplitude_dbfs,
        stereo_meter,
    },
    continuous_slider,
    eq_plot::{EqPlot, EqView},
    frequency::{Biquad, EqBand, FFT_SIZE, FilterKind},
    pill_segmented,
    waveform::{Waveform, paint_waveform},
};

const RATE: f32 = 48_000.0;
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;
const GAIN: f32 = 18.0;
const WAVE_POINTS: usize = 420;
const TABLE_SIZE: usize = 2048;
const FRAME_PERIOD: f64 = 1.0 / 60.0;
const FFT_PERIOD: f64 = 1.0 / 30.0;
const BANDS: [EqBand; 4] = [
    EqBand {
        frequency: 80.0,
        gain_db: 0.0,
        q: 0.707,
        kind: FilterKind::HighPass,
    },
    EqBand {
        frequency: 220.0,
        gain_db: -3.0,
        q: 0.707,
        kind: FilterKind::LowShelf,
    },
    EqBand {
        frequency: 1_600.0,
        gain_db: 6.0,
        q: 1.2,
        kind: FilterKind::Bell,
    },
    EqBand {
        frequency: 6_500.0,
        gain_db: -3.0,
        q: 0.707,
        kind: FilterKind::HighShelf,
    },
];

pub(super) struct Preview {
    bands: [EqBand; 4],
    compiled_bands: [EqBand; 4],
    filters: [Biquad; 4],
    selected: usize,
    view: EqView,
    playing: bool,
    gain: f32,
    meters: [ChannelLevel; 2],
    ballistics: [PeakBallistics; 2],
    brass: Brass,
    last_frame: Option<f64>,
    last_fft: Option<f64>,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            bands: BANDS,
            compiled_bands: BANDS,
            filters: BANDS.map(|band| Biquad::new(RATE, band)),
            selected: 2,
            view: EqView::Overlay,
            playing: true,
            gain: 0.0,
            meters: Default::default(),
            ballistics: Default::default(),
            brass: Brass::default(),
            last_frame: None,
            last_fft: None,
        }
    }
}

impl Preview {
    pub(super) fn playing(&self) -> bool {
        self.playing
    }

    pub(super) fn toggle_playback(&mut self) {
        self.playing = !self.playing;
        self.suspend_preview();
    }

    pub(super) fn suspend_preview(&mut self) {
        self.last_frame = None;
        self.last_fft = None;
    }

    pub(super) fn show(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.set_min_width(ui.available_width());
        ui.heading("Parametric EQ and trumpet spectrum");
        let label = ui.weak("48 kHz · FFT 2048 · logarithmic frequency");
        pill_segmented(
            ui,
            tokens,
            &label,
            &mut self.view,
            &[
                (EqView::Eq, "EQ"),
                (EqView::Spectrum, "FFT"),
                (EqView::Overlay, "Overlay"),
            ],
        );
        let plot_visible = EqPlot {
            bands: &mut self.bands,
            selected: &mut self.selected,
            band_labels: &[
                "Select band 1",
                "Select band 2",
                "Select band 3",
                "Select band 4",
            ],
            view: self.view,
            bins: &self.brass.bins,
            sample_rate: RATE,
        }
        .show_visible(ui, tokens);

        ui.horizontal_wrapped(|ui| {
            for index in 0..self.bands.len() {
                if orange_ui_controls::button(
                    ui,
                    Surface::Card,
                    orange_ui_controls::Action::Toggle(self.selected == index),
                    true,
                    egui::Button::new(format!("Band {}", index + 1)),
                )
                .clicked()
                {
                    self.selected = index;
                }
            }
            if ui.button("Reset EQ").clicked() {
                self.bands = BANDS;
            }
        });
        let band = &mut self.bands[self.selected];
        let label = ui.weak("Filter");
        pill_segmented(
            ui,
            tokens,
            &label,
            &mut band.kind,
            &[
                (FilterKind::LowShelf, "Low shelf"),
                (FilterKind::Bell, "Bell"),
                (FilterKind::HighPass, "High-pass"),
                (FilterKind::HighShelf, "High shelf"),
            ],
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut band.frequency)
                    .prefix("Frequency ")
                    .suffix(" Hz")
                    .range(MIN_HZ..=MAX_HZ)
                    .speed(5.0)
                    .max_decimals(0),
            );
            ui.add_enabled(
                band.kind != FilterKind::HighPass,
                egui::DragValue::new(&mut band.gain_db)
                    .prefix("Gain ")
                    .suffix(" dB")
                    .range(-GAIN..=GAIN)
                    .speed(0.1)
                    .fixed_decimals(1),
            );
            ui.add(
                egui::DragValue::new(&mut band.q)
                    .prefix("Q ")
                    .range(0.1..=18.0)
                    .speed(0.02)
                    .fixed_decimals(2),
            );
        });
        ui.weak("Post-EQ trumpet waveform");
        let wave_visible = waveform(ui, tokens, Waveform::Samples(&self.brass.waveform), None);

        Divider::Section.show(ui, tokens);
        ui.heading("Stereo peak and RMS · dBFS");
        let mut trigger = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if self.playing { "Pause" } else { "Play" })
                .clicked()
            {
                self.toggle_playback();
            }
            trigger = ui.button("Trigger peak").clicked();
        });
        let label = ui.label(format!("Gain {:+.1} dB", self.gain));
        if continuous_slider(ui, Surface::Card, &mut self.gain, -60.0..=6.0)
            .labelled_by(label.id)
            .on_hover_text("Double-click for 0 dB")
            .double_clicked()
        {
            self.gain = 0.0;
        }
        ui.label(format!(
            "L  RMS {:.1} / Peak {:.1}     R  RMS {:.1} / Peak {:.1}",
            self.meters[0].rms, self.meters[0].peak, self.meters[1].rms, self.meters[1].peak
        ));
        let names = ["L", "R"];
        let reset = ["Reset left clip", "Reset right clip"];
        let horizontal = stereo_meter(
            ui,
            tokens,
            MeterOrientation::Horizontal,
            MeterStyle::Continuous,
            &mut self.meters,
            names,
            reset,
        );
        let vertical = ui
            .horizontal_wrapped(|ui| {
                for style in [MeterStyle::Continuous, MeterStyle::Segmented] {
                    stereo_meter(
                        ui,
                        tokens,
                        MeterOrientation::Vertical,
                        style,
                        &mut self.meters,
                        names,
                        reset,
                    );
                }
                let (rect, response) = ui.allocate_exact_size(vec2(84.0, 184.0), Sense::hover());
                let lane = Rect::from_min_size(rect.min + vec2(12.0, 13.0), vec2(40.0, 158.0));
                RmsMeter {
                    lane,
                    zoom: 1.0,
                    reading: None,
                    target: Some((-35.0, tokens.theme.accent_primary())),
                }
                .show_channels(
                    ui,
                    ui.painter(),
                    response.id,
                    tokens,
                    &[(
                        "RMS",
                        ChannelReading {
                            rms: self.meters[0].rms,
                            peak: self.meters[0].peak,
                            clipped: self.meters[0].clipped,
                        },
                    )],
                );
            })
            .response;
        let fft_visible = plot_visible && self.view != EqView::Eq;
        let visible = fft_visible
            || wave_visible
            || ui.is_rect_visible(horizontal.rect)
            || ui.is_rect_visible(vertical.rect);
        self.update(ui, visible, fft_visible, wave_visible, trigger);
    }

    fn update(
        &mut self,
        ui: &Ui,
        visible: bool,
        fft_visible: bool,
        wave_visible: bool,
        trigger: bool,
    ) {
        if (!visible || !self.playing) && !trigger {
            self.suspend_preview();
            return;
        }
        let now = ui.input(|input| input.time);
        let elapsed = self.last_frame.map_or(FRAME_PERIOD, |before| now - before);
        if trigger || elapsed + 0.001 >= FRAME_PERIOD {
            self.last_frame = Some(now);
            let measure = fft_visible
                && self
                    .last_fft
                    .is_none_or(|last| now - last + 0.001 >= FFT_PERIOD);
            if measure {
                self.last_fft = Some(now);
            }
            if self.compiled_bands != self.bands {
                self.filters = self.bands.map(|band| Biquad::new(RATE, band));
                self.compiled_bands = self.bands;
            }
            let dt = elapsed.clamp(0.0, 0.05) as f32;
            let peaks = self.brass.advance(dt, self.gain, self.filters, trigger);
            if wave_visible {
                self.brass.update_waveform();
            }
            if measure {
                self.brass.measure_spectrum(dt);
            }
            for (index, (rms, peak)) in peaks.into_iter().enumerate() {
                self.ballistics[index].level.clipped = self.meters[index].clipped;
                self.ballistics[index].advance(rms, peak, dt);
                self.meters[index] = self.ballistics[index].level;
            }
        }
        if self.playing {
            let until = self.last_frame.map_or(FRAME_PERIOD, |before| {
                (before + FRAME_PERIOD - now).max(0.001)
            });
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(until));
        } else if trigger {
            ui.ctx().request_repaint();
        }
    }
}
struct Brass {
    phase: f32,
    time: f32,
    ring: [f32; FFT_SIZE],
    waveform: [f32; WAVE_POINTS],
    fft_input: [f32; FFT_SIZE],
    measured_bins: [f32; FFT_SIZE / 2],
    write: usize,
    filters: [[f64; 2]; 4],
    fft: Spectrum,
    bins: [f32; FFT_SIZE / 2],
    table: Box<[f32; TABLE_SIZE]>,
}

impl Default for Brass {
    fn default() -> Self {
        Self {
            phase: 0.0,
            time: 0.0,
            ring: [0.0; FFT_SIZE],
            waveform: [0.0; WAVE_POINTS],
            fft_input: [0.0; FFT_SIZE],
            measured_bins: [-120.0; FFT_SIZE / 2],
            write: 0,
            filters: [[0.0; 2]; 4],
            fft: Spectrum::default(),
            bins: [-120.0; FFT_SIZE / 2],
            table: brass_table(),
        }
    }
}

fn brass_table() -> Box<[f32; TABLE_SIZE]> {
    let weights: [f32; 16] = std::array::from_fn(|i| {
        let harmonic = (i + 1) as f32;
        let hz = harmonic * 233.08;
        let formant = 1.0 + 3.0 * (-((hz - 1_850.0) / 660.0).powi(2)).exp();
        formant / harmonic.powf(1.2)
    });
    Box::new(std::array::from_fn(|index| {
        let phase = TAU * index as f32 / TABLE_SIZE as f32;
        weights
            .iter()
            .enumerate()
            .map(|(i, weight)| weight * (phase * (i + 1) as f32).sin())
            .sum()
    }))
}
impl Brass {
    fn advance(
        &mut self,
        dt: f32,
        gain: f32,
        filters: [Biquad; 4],
        trigger: bool,
    ) -> [(f32, f32); 2] {
        let count = (dt * RATE).round().max(1.0) as usize;
        let gain = 10.0_f32.powf(gain / 20.0);
        let mut power = [0.0; 2];
        let mut peak = [0.0_f32; 2];
        for i in 0..count {
            self.time += RATE.recip();
            let note = self.time.rem_euclid(0.82);
            let envelope = (note * 45.0).min(1.0) * ((0.66 - note) * 18.0).clamp(0.0, 1.0);
            let vibrato = 1.0 + 0.006 * (TAU * 5.4 * self.time).sin();
            self.phase = (self.phase + TAU * 233.08 * vibrato / RATE).rem_euclid(TAU);
            let position = self.phase * (TABLE_SIZE as f32 / TAU);
            let index = position as usize & (TABLE_SIZE - 1);
            let mut sample = egui::lerp(
                self.table[index]..=self.table[(index + 1) & (TABLE_SIZE - 1)],
                position.fract(),
            );
            sample *= 0.37 * envelope * (0.82 + 0.18 * (self.time * 8.0).sin());
            if trigger && i < 12 {
                sample += if i % 2 == 0 { 2.0 } else { -2.0 };
            }
            for (filter, state) in filters.iter().zip(&mut self.filters) {
                sample = filter.process(sample, state);
            }
            self.ring[self.write] = sample * gain;
            self.write = (self.write + 1) % FFT_SIZE;
            let pan = 0.10 * (self.time * 2.1).sin();
            for (index, sample) in [sample * gain * (0.88 + pan), sample * gain * (0.76 - pan)]
                .into_iter()
                .enumerate()
            {
                peak[index] = peak[index].max(sample.abs());
                power[index] += sample * sample;
            }
        }
        if trigger {
            peak[0] = peak[0].max(1.15);
            peak[1] = peak[1].max(1.06);
        }
        std::array::from_fn(|index| {
            (
                amplitude_dbfs((power[index] / count as f32).sqrt()),
                amplitude_dbfs(peak[index]),
            )
        })
    }
    fn update_waveform(&mut self) {
        for (i, sample) in self.waveform.iter_mut().enumerate() {
            *sample = self.ring[(self.write + FFT_SIZE - WAVE_POINTS + i) % FFT_SIZE];
        }
    }

    fn measure_spectrum(&mut self, dt: f32) {
        let first = FFT_SIZE - self.write;
        self.fft_input[..first].copy_from_slice(&self.ring[self.write..]);
        self.fft_input[first..].copy_from_slice(&self.ring[..self.write]);
        self.fft.measure(&self.fft_input, &mut self.measured_bins);
        let release = 105.0 * dt.max(FFT_PERIOD as f32);
        for (shown, &measured) in self.bins.iter_mut().zip(&self.measured_bins) {
            *shown = measured.max(*shown - release);
        }
    }
}

pub(super) fn waveform(
    ui: &mut Ui,
    tokens: UiTokens,
    data: Waveform<'_>,
    playhead: Option<&mut f32>,
) -> bool {
    let sense = if playhead.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), tokens.density.row_height() * 2.3),
        sense,
    );
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let theme = tokens.theme;
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        theme.radius,
        theme.bg_canvas(),
        theme.border_subtle(),
        StrokeKind::Inside,
    );
    let plot = rect.shrink(4.0);
    painter.hline(plot.x_range(), plot.center().y, theme.border_subtle());
    paint_waveform(&painter, plot, data, theme.accent_primary());
    if let Some(position) = playhead {
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            *position = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        }
        let x = egui::lerp(plot.x_range(), *position);
        painter.vline(x, plot.y_range(), Stroke::new(2.0, theme.text_primary()));
        painter.circle_filled(pos2(x, plot.center().y), 3.5, theme.text_primary());
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    true
}

#[cfg(test)]
mod scheduling_tests {
    use super::*;

    #[test]
    fn hidden_paused_and_early_frames_skip_signal_work() {
        let ctx = egui::Context::default();
        let mut preview = Preview::default();
        let mut frame = |time, visible, fft, playing| {
            preview.playing = playing;
            ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    preview.update(ui, visible, fft, false, false);
                },
            )
            .drop_without_applying_deltas();
            (preview.brass.write, preview.last_fft)
        };
        assert_eq!(frame(0.0, false, false, true), (0, None));
        let first = frame(0.1, true, true, true);
        assert_eq!(first.1, Some(0.1));
        assert_eq!(frame(0.11, true, true, true), first);
        let second = frame(0.12, true, true, true);
        assert_ne!(second.0, first.0);
        assert_eq!(second.1, first.1);
        assert_eq!(frame(0.3, true, true, false), (second.0, None));
        assert_eq!(frame(0.8, false, false, true), (second.0, None));
    }
    #[test]
    fn profile_audio_panel() {
        for dragging in [false, true] {
            measure_panel(dragging);
        }
    }
    fn measure_panel(dragging: bool) {
        let ctx = egui::Context::default();
        let mut preview = Preview::default();
        let mut times = Vec::with_capacity(600);
        for frame in 0..660 {
            let start = std::time::Instant::now();
            if dragging {
                preview.bands[2].frequency = 1600.0 + 1000.0 * (frame as f32 * 0.03).sin();
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 1000.0))),
                    time: Some(f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |ui| preview.show(ui, UiTokens::resolve(&ctx)),
            );
            std::hint::black_box(
                ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point),
            );
            output.drop_without_applying_deltas();
            if frame >= 60 {
                times.push(start.elapsed().as_micros());
            }
        }
        times.sort_unstable();
        println!(
            "600 audio panel frames (changing band = {dragging}), CPU microseconds: median {} p95 {} p99 {}",
            times[300], times[570], times[594]
        );
    }
    #[test]
    fn profile_audio_math() {
        let mut brass = Brass::default();
        let filters = BANDS.map(|band| Biquad::new(RATE, band));
        let mut times = Vec::with_capacity(600);
        for frame in 0..660 {
            let start = std::time::Instant::now();
            std::hint::black_box(brass.advance(FRAME_PERIOD as f32, 0.0, filters, false));
            brass.update_waveform();
            brass.measure_spectrum(FFT_PERIOD as f32);
            std::hint::black_box((&brass.waveform, &brass.bins));
            if frame >= 60 {
                times.push(start.elapsed().as_micros());
            }
        }
        times.sort_unstable();
        println!(
            "600 signal blocks including FFT, CPU microseconds: median {} p95 {} p99 {}",
            times[300], times[570], times[594]
        );
    }
}

/// The gallery's live analyser: a Hann-windowed transform of the last `FFT_SIZE`
/// samples, allocation-free after it is made, through the audio package's FFT. Bin `i`
/// is `RATE * i / FFT_SIZE`; the output is single-sided peak amplitude in dBFS.
#[derive(Debug)]
struct Spectrum {
    window: Vec<f64>,
    twiddles: Vec<(f64, f64)>,
    data: Vec<(f64, f64)>,
}

impl Default for Spectrum {
    #[expect(clippy::cast_precision_loss, reason = "a small transform size")]
    fn default() -> Self {
        Self {
            window: (0..FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FFT_SIZE as f64).cos())
                .collect(),
            twiddles: orange_audio::fft::twiddles(FFT_SIZE),
            data: vec![(0.0, 0.0); FFT_SIZE],
        }
    }
}

impl Spectrum {
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "a small transform size and display levels"
    )]
    fn measure(&mut self, samples: &[f32; FFT_SIZE], output: &mut [f32; FFT_SIZE / 2]) {
        for (i, (&sample, &window)) in samples.iter().zip(&self.window).enumerate() {
            self.data[orange_audio::fft::reversed(i, FFT_SIZE)] = (f64::from(sample) * window, 0.0);
        }
        orange_audio::fft::transform(&mut self.data, &self.twiddles);
        for (i, (out, &(real, imag))) in output.iter_mut().zip(&self.data).enumerate() {
            let scale = if i == 0 { 2.0 } else { 4.0 };
            *out = amplitude_dbfs((real.hypot(imag) * scale / FFT_SIZE as f64) as f32);
        }
    }
}
