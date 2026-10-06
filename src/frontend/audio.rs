//! Salida de audio: anillo sin bloqueos entre el hilo de emulación (productor) y el callback del
//! dispositivo (consumidor), control de volumen/silencio y mezcla de beeper + sonido de cinta.
//!
//! El core entrega transiciones con su T-state; el remuestreador determinista del core las
//! convierte en muestras. Esta capa solo transporta, mezcla y reproduce.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Ganancia del beeper antes del volumen maestro (evita saturar la salida).
pub const BEEPER_GAIN: f32 = 0.30;
/// Ganancia del sonido de la señal de cinta ("sonido de carga"), más suave que el beeper.
pub const TAPE_GAIN: f32 = 0.18;
/// Paso máximo de la ganancia por muestra al cambiar volumen/silencio (evita chasquidos).
const GAIN_STEP: f32 = 1.0 / 2048.0;

/// Parámetros compartidos entre la interfaz y el callback de audio.
#[derive(Debug)]
pub struct AudioControl {
    volume_bits: AtomicU32,
    muted: AtomicBool,
    failed: AtomicBool,
}

impl AudioControl {
    pub fn new(volume: f32, muted: bool) -> Self {
        let c = Self {
            volume_bits: AtomicU32::new(0),
            muted: AtomicBool::new(muted),
            failed: AtomicBool::new(false),
        };
        c.set_volume(volume);
        c
    }

    pub fn set_volume(&self, v: f32) {
        let v = if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.volume_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_muted(&self, m: bool) {
        self.muted.store(m, Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    /// El dispositivo ha dejado de funcionar (p. ej. desconectado).
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Ganancia efectiva: curva cuadrática (más natural al oído que la lineal) o 0 en silencio.
    pub fn target_gain(&self) -> f32 {
        if self.muted() {
            0.0
        } else {
            self.volume() * self.volume()
        }
    }
}

/// Mezcla beeper y señal de cinta en `out` (sustituye su contenido).
pub fn mix(out: &mut Vec<f32>, speaker: &[f32], tape: &[f32], tape_enabled: bool) {
    out.clear();
    if tape_enabled && tape.len() == speaker.len() {
        out.extend(
            speaker
                .iter()
                .zip(tape)
                .map(|(s, t)| s * BEEPER_GAIN + t * TAPE_GAIN),
        );
    } else {
        out.extend(speaker.iter().map(|s| s * BEEPER_GAIN));
    }
}

/// Extremo productor del anillo (lo posee el hilo de emulación).
pub struct AudioProducer {
    prod: HeapProd<f32>,
    rate: u32,
}

impl AudioProducer {
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Muestras pendientes de reproducir.
    pub fn fill(&self) -> usize {
        self.prod.occupied_len()
    }

    /// Añade muestras; devuelve cuántas cupieron (el resto se descarta, nunca se bloquea).
    pub fn push(&mut self, samples: &[f32]) -> usize {
        self.prod.push_slice(samples)
    }
}

/// Extremo consumidor: rellena los buffers del dispositivo.
pub struct Renderer {
    cons: HeapCons<f32>,
    channels: usize,
    gain: f32,
    control: Arc<AudioControl>,
}

impl Renderer {
    /// Rellena `out` (intercalado, `channels` por fotograma). Si faltan muestras, silencio.
    pub fn fill<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T]) {
        let target = self.control.target_gain();
        for frame in out.chunks_mut(self.channels.max(1)) {
            let s = self.cons.try_pop().unwrap_or(0.0);
            if self.gain < target {
                self.gain = (self.gain + GAIN_STEP).min(target);
            } else if self.gain > target {
                self.gain = (self.gain - GAIN_STEP).max(target);
            }
            let v = T::from_sample((s * self.gain).clamp(-1.0, 1.0));
            frame.iter_mut().for_each(|o| *o = v);
        }
    }
}

/// Crea el anillo (capacidad en muestras) y sus dos extremos.
pub fn ring(
    rate: u32,
    capacity: usize,
    channels: usize,
    control: Arc<AudioControl>,
) -> (AudioProducer, Renderer) {
    let (prod, cons) = HeapRb::<f32>::new(capacity.max(1)).split();
    (
        AudioProducer { prod, rate },
        Renderer {
            cons,
            channels,
            gain: 0.0,
            control,
        },
    )
}

/// Dispositivo abierto. Debe permanecer vivo (en el hilo que lo creó) mientras suene el audio.
pub struct AudioOutput {
    _stream: cpal::Stream,
    pub control: Arc<AudioControl>,
    pub sample_rate: u32,
    pub device_name: String,
}

impl AudioOutput {
    /// Abre el dispositivo de salida por defecto. Errores en forma de mensaje comprensible.
    pub fn open(volume: f32, muted: bool) -> Result<(AudioOutput, AudioProducer), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no hay dispositivo de audio de salida")?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("no se pudo consultar la configuración de audio: {e}"))?;
        let format = supported.sample_format();
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let config: cpal::StreamConfig = supported.into();
        let control = Arc::new(AudioControl::new(volume, muted));
        // Un segundo de capacidad: la emulación mantiene el nivel en pocos fotogramas.
        let (producer, renderer) = ring(rate, rate as usize, channels, control.clone());
        let err_control = control.clone();
        let on_err = move |e: cpal::Error| {
            tracing::warn!(target: "zx48::audio", "error del dispositivo de audio: {e}");
            err_control.failed.store(true, Ordering::Relaxed);
        };
        let stream = match format {
            cpal::SampleFormat::F32 => {
                let mut r = renderer;
                device.build_output_stream(config, move |d: &mut [f32], _| r.fill(d), on_err, None)
            }
            cpal::SampleFormat::I16 => {
                let mut r = renderer;
                device.build_output_stream(config, move |d: &mut [i16], _| r.fill(d), on_err, None)
            }
            cpal::SampleFormat::U16 => {
                let mut r = renderer;
                device.build_output_stream(config, move |d: &mut [u16], _| r.fill(d), on_err, None)
            }
            other => return Err(format!("formato de muestra no soportado: {other}")),
        }
        .map_err(|e| format!("no se pudo abrir el dispositivo de audio: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("no se pudo iniciar el audio: {e}"))?;
        let device_name = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "dispositivo".into());
        Ok((
            AudioOutput {
                _stream: stream,
                control,
                sample_rate: rate,
                device_name,
            },
            producer,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(cap: usize, channels: usize) -> (AudioProducer, Renderer, Arc<AudioControl>) {
        let c = Arc::new(AudioControl::new(1.0, false));
        let (p, r) = ring(44_100, cap, channels, c.clone());
        (p, r, c)
    }

    #[test]
    fn volume_is_clamped_and_gain_is_monotonic() {
        let c = AudioControl::new(5.0, false);
        assert_eq!(c.volume(), 1.0);
        c.set_volume(-1.0);
        assert_eq!(c.volume(), 0.0);
        c.set_volume(f32::NAN);
        assert_eq!(c.volume(), 0.0);
        let mut last = -1.0;
        for i in 0..=10 {
            c.set_volume(i as f32 / 10.0);
            let g = c.target_gain();
            assert!(g > last || i == 0);
            last = g;
        }
        assert_eq!(c.target_gain(), 1.0);
        c.set_muted(true);
        assert_eq!(c.target_gain(), 0.0);
        assert!(c.muted());
    }

    #[test]
    fn mixing_scales_and_combines() {
        let mut out = Vec::new();
        mix(&mut out, &[0.5, -0.5], &[1.0, 1.0], false);
        assert_eq!(out, vec![0.5 * BEEPER_GAIN, -0.5 * BEEPER_GAIN]);
        mix(&mut out, &[0.5, -0.5], &[1.0, 1.0], true);
        assert_eq!(
            out,
            vec![
                0.5 * BEEPER_GAIN + TAPE_GAIN,
                -0.5 * BEEPER_GAIN + TAPE_GAIN
            ]
        );
        // Longitudes distintas: se ignora la cinta en lugar de desalinear.
        mix(&mut out, &[0.5, 0.5, 0.5], &[1.0], true);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2], 0.5 * BEEPER_GAIN);
        assert!(
            BEEPER_GAIN + TAPE_GAIN <= 0.5 * 2.0,
            "la suma no satura ni a máxima amplitud"
        );
    }

    #[test]
    fn producer_never_blocks_and_reports_fill() {
        let (mut p, mut r, _) = pair(8, 1);
        assert_eq!(p.fill(), 0);
        assert_eq!(p.push(&[0.1; 5]), 5);
        assert_eq!(p.fill(), 5);
        assert_eq!(p.push(&[0.1; 10]), 3, "lo que no cabe se descarta");
        assert_eq!(p.fill(), 8);
        let mut out = [0.0f32; 4];
        r.fill(&mut out);
        assert_eq!(p.fill(), 4);
        assert_eq!(p.sample_rate(), 44_100);
    }

    #[test]
    fn renderer_plays_samples_in_order_and_duplicates_channels() {
        let (mut p, mut r, _) = pair(64, 2);
        r.gain = 1.0; // sin rampa inicial
        p.push(&[0.1, 0.2, 0.3]);
        let mut out = [0.0f32; 8];
        r.fill(&mut out);
        assert_eq!(&out[..6], &[0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
        assert_eq!(&out[6..], &[0.0, 0.0], "subdesbordamiento = silencio");
    }

    #[test]
    fn renderer_converts_to_integer_formats() {
        let (mut p, mut r, _) = pair(8, 1);
        r.gain = 1.0;
        p.push(&[1.0, -1.0, 0.0]);
        let mut out = [7i16; 3];
        r.fill(&mut out);
        assert!(
            out[0] > 32_000 && out[1] < -32_000 && out[2].abs() <= 1,
            "{out:?}"
        );
        let (mut p, mut r, _) = pair(8, 1);
        r.gain = 1.0;
        p.push(&[0.0]);
        let mut out = [0u16; 1];
        r.fill(&mut out);
        assert!((out[0] as i32 - 32_768).abs() <= 1, "{out:?}");
    }

    #[test]
    fn mute_and_volume_changes_ramp_instead_of_clicking() {
        let (mut p, mut r, c) = pair(1 << 14, 1);
        r.gain = 1.0;
        p.push(&vec![1.0; 6000]);
        c.set_muted(true);
        let mut out = vec![0.0f32; 4096];
        r.fill(&mut out);
        // Rampa descendente: nunca un salto brusco entre muestras consecutivas.
        assert!(
            out.windows(2)
                .all(|w| (w[0] - w[1]).abs() <= GAIN_STEP + 1e-6)
        );
        assert!(out[0] > 0.9 && *out.last().unwrap() < 1e-3);
        c.set_muted(false);
        let mut out = vec![0.0f32; 1500];
        r.fill(&mut out);
        assert!(
            out[0] < 0.01 && out.windows(2).all(|w| w[1] >= w[0] - 1e-6),
            "sube suavemente"
        );
    }

    #[test]
    fn output_is_always_within_range() {
        let (mut p, mut r, _) = pair(16, 1);
        r.gain = 1.0;
        p.push(&[5.0, -5.0]);
        let mut out = [0.0f32; 2];
        r.fill(&mut out);
        assert_eq!(out, [1.0, -1.0]);
    }
}
