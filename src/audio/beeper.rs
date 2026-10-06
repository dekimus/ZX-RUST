//! Altavoz de 1 bit (bit 4 del puerto 0xFE). El core solo registra transiciones con su
//! T-state; la conversión a muestras es un remuestreo determinista, también sin reloj de pared.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioEvent {
    /// T-state maestro en el que cambia el nivel.
    pub tstate: u64,
    pub level: bool,
}

#[derive(Default, Debug)]
pub struct Beeper {
    level: bool,
    events: Vec<AudioEvent>,
}

impl Beeper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn level(&self) -> bool {
        self.level
    }

    /// Registra el nivel; solo las transiciones generan evento.
    pub fn set_level(&mut self, tstate: u64, level: bool) {
        if level != self.level {
            self.level = level;
            tracing::trace!(target: "zx48::audio", tstate, event = "SPEAKER", level);
            self.events.push(AudioEvent { tstate, level });
        }
    }

    /// Entrega y vacía los eventos acumulados (el frontend debe llamarlo periódicamente).
    pub fn drain_events(&mut self) -> Vec<AudioEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Convierte transiciones en muestras promediando el nivel durante cada intervalo de muestra
/// (filtro de caja). Aritmética entera para las fronteras: totalmente determinista.
#[derive(Debug)]
pub struct Resampler {
    cpu_hz: u64,
    rate: u64,
    next_sample: u64,
    cursor: u64,
    level: bool,
    acc: u64,
    pending: VecDeque<AudioEvent>,
}

impl Resampler {
    pub fn new(cpu_hz: u32, sample_rate: u32) -> Self {
        Self {
            cpu_hz: cpu_hz as u64,
            rate: sample_rate as u64,
            next_sample: 0,
            cursor: 0,
            level: false,
            acc: 0,
            pending: VecDeque::new(),
        }
    }

    fn boundary(&self, n: u64) -> u64 {
        (n as u128 * self.cpu_hz as u128 / self.rate as u128) as u64
    }

    /// Salta a `tstate` descartando eventos pendientes y manteniendo `level`. Debe usarse cuando
    /// el reloj de la máquina cambia de golpe (reset, carga de snapshot) o se ha dejado de
    /// consumir audio (turbo); si no, el remuestreador intentaría rellenar todo el intervalo.
    pub fn skip_to(&mut self, tstate: u64, level: bool) {
        let n = (tstate as u128 * self.rate as u128).div_ceil(self.cpu_hz as u128) as u64;
        self.next_sample = n;
        self.cursor = self.boundary(n);
        self.level = level;
        self.acc = 0;
        self.pending.clear();
    }

    /// Añade eventos (en orden) y genera muestras completas hasta `until` (T-state maestro).
    /// Las muestras salen centradas en 0 (rango -0.5..=0.5).
    pub fn process(&mut self, events: &[AudioEvent], until: u64, out: &mut Vec<f32>) {
        self.pending.extend(events.iter().copied());
        loop {
            let start = self.boundary(self.next_sample);
            let end = self.boundary(self.next_sample + 1);
            if end > until {
                break;
            }
            while let Some(e) = self.pending.front().copied() {
                if e.tstate >= end {
                    break;
                }
                let t = e.tstate.max(self.cursor);
                self.acc += self.level as u64 * (t - self.cursor);
                self.cursor = t;
                self.level = e.level;
                self.pending.pop_front();
            }
            self.acc += self.level as u64 * (end - self.cursor);
            self.cursor = end;
            let len = (end - start).max(1);
            out.push(self.acc as f32 / len as f32 - 0.5);
            self.acc = 0;
            self.next_sample += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transitions_generate_events() {
        let mut b = Beeper::new();
        b.set_level(10, false);
        b.set_level(20, true);
        b.set_level(30, true);
        b.set_level(40, false);
        assert_eq!(
            b.drain_events(),
            vec![
                AudioEvent {
                    tstate: 20,
                    level: true
                },
                AudioEvent {
                    tstate: 40,
                    level: false
                }
            ]
        );
        assert!(b.drain_events().is_empty());
    }

    #[test]
    fn constant_levels_resample_to_constant() {
        let mut r = Resampler::new(3_500_000, 35_000); // 100 T por muestra
        let mut out = Vec::new();
        r.process(&[], 1000, &mut out);
        assert_eq!(out.len(), 10);
        assert!(out.iter().all(|&s| s == -0.5));
        r.process(
            &[AudioEvent {
                tstate: 1000,
                level: true,
            }],
            2000,
            &mut out,
        );
        assert_eq!(out.len(), 20);
        assert!(out[10..].iter().all(|&s| s == 0.5));
    }

    #[test]
    fn square_wave_averages_to_zero_and_edge_splits_sample() {
        let mut r = Resampler::new(3_500_000, 35_000);
        let mut out = Vec::new();
        // Nivel alto desde T=150 (mitad de la muestra 1) hasta T=250 (mitad de la muestra 2).
        let ev = [
            AudioEvent {
                tstate: 150,
                level: true,
            },
            AudioEvent {
                tstate: 250,
                level: false,
            },
        ];
        r.process(&ev, 400, &mut out);
        assert_eq!(out, vec![-0.5, 0.0, 0.0, -0.5]);
    }

    #[test]
    fn events_after_until_are_kept_for_later() {
        let mut r = Resampler::new(3_500_000, 35_000);
        let mut out = Vec::new();
        r.process(
            &[AudioEvent {
                tstate: 250,
                level: true,
            }],
            150,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        r.process(&[], 300, &mut out);
        assert_eq!(out, vec![-0.5, -0.5, 0.0]);
    }

    #[test]
    fn skip_to_resynchronises_without_flooding_samples() {
        let mut r = Resampler::new(3_500_000, 44_100);
        let mut out = Vec::new();
        r.process(&[], 70_000, &mut out);
        let before = out.len();
        // Salto de 10 s hacia delante: no debe generar 10 s de muestras.
        r.skip_to(35_000_000, true);
        r.process(&[], 35_000_000 + 7_000, &mut out);
        let produced = out.len() - before;
        // 7000 T = ~88 muestras; no los 10 s (441 000) de salto.
        assert!((86..=89).contains(&produced), "producidas {produced}");
        assert!(
            out[before..].iter().all(|&s| s == 0.5),
            "mantiene el nivel indicado"
        );
        // Salto hacia atrás (reset): vuelve a producir con normalidad.
        r.skip_to(0, false);
        let n = out.len();
        r.process(&[], 70_000, &mut out);
        assert!((out.len() - n) as i64 - 882 <= 1);
        assert!(out[n..].iter().all(|&s| s == -0.5));
    }

    #[test]
    fn fractional_sample_boundaries_are_deterministic() {
        let run = || {
            let mut r = Resampler::new(3_500_000, 44_100);
            let mut out = Vec::new();
            let ev: Vec<_> = (0..200)
                .map(|i| AudioEvent {
                    tstate: i * 997,
                    level: i % 2 == 0,
                })
                .collect();
            r.process(&ev, 69_888 * 3, &mut out);
            out
        };
        assert_eq!(run(), run());
        assert_eq!(run().len(), (69_888u64 * 3 * 44_100 / 3_500_000) as usize);
    }
}
