use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering::Relaxed},
};

use cpal::{
    FromSample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

const NO_SEEK: u32 = u32::MAX;

struct Transport {
    seek: AtomicU32,
    position: AtomicU32,
    volume: AtomicU32,
    fault: AtomicBool,
}

pub(super) struct Playback {
    _stream: cpal::Stream,
    transport: Arc<Transport>,
}

impl Playback {
    pub(super) fn start(
        samples: Arc<[f32]>,
        rate: u32,
        fraction: f32,
        volume: f32,
    ) -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No default audio output.")?;
        let default = device
            .default_output_config()
            .map_err(|error| error.to_string())?;
        let format = default.sample_format();
        let config = default.config();
        let transport = Arc::new(Transport {
            seek: AtomicU32::new(fraction.to_bits()),
            position: AtomicU32::new(fraction.to_bits()),
            volume: AtomicU32::new(volume.to_bits()),
            fault: AtomicBool::new(false),
        });
        macro_rules! stream {
            ($sample:ty) => {
                build::<$sample>(&device, config, samples, rate, transport.clone())
            };
        }
        let stream = match format {
            SampleFormat::I8 => stream!(i8),
            SampleFormat::I16 => stream!(i16),
            SampleFormat::I24 => stream!(cpal::I24),
            SampleFormat::I32 => stream!(i32),
            SampleFormat::I64 => stream!(i64),
            SampleFormat::U8 => stream!(u8),
            SampleFormat::U16 => stream!(u16),
            SampleFormat::U32 => stream!(u32),
            SampleFormat::U64 => stream!(u64),
            SampleFormat::F32 => stream!(f32),
            SampleFormat::F64 => stream!(f64),
            _ => return Err(format!("Audio output format {format} is unsupported.")),
        }
        .map_err(|error| error.to_string())?;
        stream.play().map_err(|error| error.to_string())?;
        Ok(Self {
            _stream: stream,
            transport,
        })
    }

    pub(super) fn seek(&self, fraction: f32) {
        self.transport.position.store(fraction.to_bits(), Relaxed);
        self.transport.seek.store(fraction.to_bits(), Relaxed);
    }

    pub(super) fn position(&self) -> f32 {
        f32::from_bits(self.transport.position.load(Relaxed))
    }

    pub(super) fn fault(&self) -> bool {
        self.transport.fault.load(Relaxed)
    }

    pub(super) fn set_volume(&self, volume: f32) {
        self.transport.volume.store(volume.to_bits(), Relaxed);
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Arc<[f32]>,
    rate: u32,
    transport: Arc<Transport>,
) -> Result<cpal::Stream, cpal::Error> {
    let stride = f64::from(rate) / f64::from(config.sample_rate);
    let channels = usize::from(config.channels);
    let length = samples.len() as f64;
    let mut position = 0.0;
    let fault = transport.clone();
    device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let seek = transport.seek.swap(NO_SEEK, Relaxed);
            if seek != NO_SEEK {
                position = f64::from(f32::from_bits(seek)) * length;
            }
            let gain = f32::from_bits(transport.volume.load(Relaxed)) * 0.25;
            for frame in output.chunks_exact_mut(channels) {
                let index = position as usize;
                let sample = samples.get(index).map_or(0.0, |&first| {
                    let second = samples.get(index + 1).copied().unwrap_or(first);
                    egui::lerp(first..=second, position.fract() as f32) * gain
                });
                frame.fill(T::from_sample(sample));
                position = (position + stride).min(length);
            }
            transport
                .position
                .store(((position / length) as f32).to_bits(), Relaxed);
        },
        move |_| {
            fault.fault.store(true, Relaxed);
        },
        None,
    )
}
