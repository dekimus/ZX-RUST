//! Generación temporal de la señal EAR (pulsos en T-states) para TAP y TZX.
//!
//! Modelo común: un `SignalBlock` describe un trozo de cinta. Cada pulso es un cambio de nivel
//! seguido de una espera. Un bloque con pausa (`pause > 0`) termina con un flanco adicional
//! (cierra el último pulso, necesario para que la ROM detecte el final del último bit) y
//! después queda en silencio; sin pausa, el flanco inicial del siguiente bloque cierra el pulso.
//!
//! Temporización estándar de la ROM: piloto 2168 T (8063 pulsos con flag < 0x80, 3223 en datos),
//! sincronismos 667 + 735 T, bit 0 = 2×855 T, bit 1 = 2×1710 T, pausa de 1 s en TAP.

use super::tap::{Tape, TapeBlock};
use crate::audio::beeper::AudioEvent;
use crate::ula::timing::CPU_HZ;

pub const PILOT_PULSE: u32 = 2168;
pub const PILOT_HEADER_PULSES: u32 = 8063;
pub const PILOT_DATA_PULSES: u32 = 3223;
pub const SYNC1_PULSE: u32 = 667;
pub const SYNC2_PULSE: u32 = 735;
pub const ZERO_PULSE: u32 = 855;
pub const ONE_PULSE: u32 = 1710;
/// Pausa tras cada bloque TAP (1 segundo).
pub const PAUSE_TSTATES: u64 = CPU_HZ as u64;
/// T-states por milisegundo (las pausas de TZX vienen en ms).
pub const TSTATES_PER_MS: u64 = CPU_HZ as u64 / 1000;
/// Tope de bloques de control (bucles/saltos) consecutivos sin reproducir señal.
const MAX_CONTROL_STEPS: u32 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalBlock {
    /// Bloque de datos (TZX 0x10/0x11/0x14 y bloques TAP).
    Data {
        /// (duración del pulso, número de pulsos) del tono piloto.
        pilot: Option<(u32, u32)>,
        /// Pulsos de sincronismo.
        sync: Option<(u32, u32)>,
        zero: u32,
        one: u32,
        /// Bits usados del último byte (1..=8).
        used_bits: u8,
        data: Vec<u8>,
        pause: u64,
    },
    /// Tono puro (TZX 0x12).
    Tone {
        len: u32,
        count: u32,
    },
    /// Secuencia de pulsos (TZX 0x13).
    Pulses(Vec<u32>),
    /// Grabación directa (TZX 0x15): un bit por muestra de `tstates` T-states.
    Direct {
        tstates: u32,
        used_bits: u8,
        data: Vec<u8>,
        pause: u64,
    },
    /// Silencio; 0 = detener la cinta (TZX 0x20 y 0x2A).
    Pause(u64),
    /// Fija el nivel de la señal (TZX 0x2B).
    SetLevel(bool),
    LoopStart(u16),
    LoopEnd,
    /// Salto relativo en bloques, contado desde el propio bloque de salto (TZX 0x23).
    Jump(i32),
    /// Bloque informativo: conserva la numeración de bloques para los saltos.
    Nop,
}

/// Pulsos de un bloque listo para reproducir.
struct Rendered {
    pulses: Vec<u32>,
    pause: u64,
    /// Nivel que debe tener la señal tras el primer flanco (grabación directa).
    start_level: Option<bool>,
}

fn push_bits(out: &mut Vec<u32>, data: &[u8], used_bits: u8, zero: u32, one: u32) {
    let last = data.len().saturating_sub(1);
    for (i, &byte) in data.iter().enumerate() {
        let nbits = if i == last { used_bits.clamp(1, 8) } else { 8 };
        for bit in (8 - nbits..8).rev() {
            let d = if byte >> bit & 1 != 0 { one } else { zero };
            out.push(d);
            out.push(d);
        }
    }
}

impl SignalBlock {
    /// Bloque estándar de TAP / TZX 0x10: tono piloto según el flag, sincronismos y bits ROM.
    pub fn standard(data: Vec<u8>, pause: u64) -> Self {
        let pilot_count = match data.first() {
            Some(&f) if f >= 0x80 => PILOT_DATA_PULSES,
            _ => PILOT_HEADER_PULSES,
        };
        Self::Data {
            pilot: Some((PILOT_PULSE, pilot_count)),
            sync: Some((SYNC1_PULSE, SYNC2_PULSE)),
            zero: ZERO_PULSE,
            one: ONE_PULSE,
            used_bits: 8,
            data,
            pause,
        }
    }

    fn render(&self) -> Option<Rendered> {
        match self {
            Self::Data {
                pilot,
                sync,
                zero,
                one,
                used_bits,
                data,
                pause,
            } => {
                if data.is_empty() {
                    return Some(Rendered {
                        pulses: Vec::new(),
                        pause: *pause,
                        start_level: None,
                    });
                }
                let mut p = Vec::with_capacity(data.len() * 16 + 16);
                if let Some((len, count)) = pilot {
                    p.extend(std::iter::repeat_n(*len, *count as usize));
                }
                if let Some((s1, s2)) = sync {
                    p.push(*s1);
                    p.push(*s2);
                }
                push_bits(&mut p, data, *used_bits, *zero, *one);
                Some(Rendered {
                    pulses: p,
                    pause: *pause,
                    start_level: None,
                })
            }
            Self::Tone { len, count } => Some(Rendered {
                pulses: vec![*len; *count as usize],
                pause: 0,
                start_level: None,
            }),
            Self::Pulses(p) => Some(Rendered {
                pulses: p.clone(),
                pause: 0,
                start_level: None,
            }),
            Self::Direct {
                tstates,
                used_bits,
                data,
                pause,
            } => {
                let mut runs: Vec<u32> = Vec::new();
                let mut start = None;
                let mut cur = false;
                let last = data.len().saturating_sub(1);
                for (i, &byte) in data.iter().enumerate() {
                    let nbits = if i == last {
                        (*used_bits).clamp(1, 8)
                    } else {
                        8
                    };
                    for bit in (8 - nbits..8).rev() {
                        let l = byte >> bit & 1 != 0;
                        if start.is_none() {
                            start = Some(l);
                            cur = l;
                            runs.push(*tstates);
                        } else if l == cur {
                            *runs.last_mut()? += *tstates;
                        } else {
                            cur = l;
                            runs.push(*tstates);
                        }
                    }
                }
                Some(Rendered {
                    pulses: runs,
                    pause: *pause,
                    start_level: start,
                })
            }
            _ => None,
        }
    }
}

/// Pulsos de un bloque TAP estándar (sin la pausa). Bloque vacío: ninguno.
pub fn block_pulses(block: &TapeBlock) -> Vec<u32> {
    SignalBlock::standard(block.0.clone(), 0)
        .render()
        .map(|r| r.pulses)
        .unwrap_or_default()
}

/// Algo reproducible por el `TapePlayer`: cualquier formato se reduce a bloques de señal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Playable(pub Vec<SignalBlock>);

impl From<Tape> for Playable {
    fn from(t: Tape) -> Self {
        Self(
            t.blocks
                .into_iter()
                .map(|b| SignalBlock::standard(b.0, PAUSE_TSTATES))
                .collect(),
        )
    }
}

impl From<Vec<SignalBlock>> for Playable {
    fn from(v: Vec<SignalBlock>) -> Self {
        Self(v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Pulses,
    Pause,
}

enum Action {
    Load(Rendered),
    Pause(u64),
    Stop,
    SetLevel(bool),
    Next,
    LoopStart(u16),
    LoopEnd,
    Jump(i32),
}

#[derive(Debug, Default)]
pub struct TapePlayer {
    blocks: Option<Vec<SignalBlock>>,
    /// Bloque actual (o el siguiente a reproducir si está detenida).
    block: usize,
    pulses: Vec<u32>,
    idx: usize,
    cur_pause: u64,
    phase: Option<Phase>,
    playing: bool,
    level: bool,
    /// Pila de bucles: (primer bloque del cuerpo, repeticiones restantes).
    loops: Vec<(usize, u32)>,
    /// El último pulso emitido aún no ha sido cerrado por un flanco.
    open_pulse: bool,
    /// T-state maestro del próximo flanco.
    next_edge: u64,
    /// Flancos ya generados, con su T-state real (para el sonido de carga).
    edges: Vec<AudioEvent>,
}

impl TapePlayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, tape: impl Into<Playable>) {
        *self = Self {
            blocks: Some(tape.into().0),
            ..Self::default()
        };
    }

    pub fn eject(&mut self) {
        *self = Self::default();
    }

    pub fn has_tape(&self) -> bool {
        self.blocks.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Índice del bloque actual / siguiente.
    pub fn block_index(&self) -> usize {
        self.block
    }

    /// Número total de bloques de la cinta insertada (0 si no hay).
    pub fn block_count(&self) -> usize {
        self.blocks.as_ref().map_or(0, |b| b.len())
    }

    /// ¿Quedan bloques por reproducir?
    pub fn at_end(&self) -> bool {
        self.blocks.as_ref().is_none_or(|t| self.block >= t.len())
    }

    pub fn rewind(&mut self) {
        self.playing = false;
        self.block = 0;
        self.phase = None;
        self.level = false;
        self.loops.clear();
        self.open_pulse = false;
    }

    pub fn stop(&mut self) {
        tracing::debug!(target: "zx48::tape", event = "TAPE_STOP", block = self.block);
        self.playing = false;
        self.phase = None;
    }

    /// Empieza (o reanuda desde el inicio del bloque actual) en el T-state `now`.
    pub fn play(&mut self, now: u64) {
        if self.at_end() {
            return;
        }
        tracing::debug!(target: "zx48::tape", tstate = now, event = "TAPE_PLAY", block = self.block);
        self.playing = true;
        self.phase = None;
        self.open_pulse = false;
        self.start_next(now);
    }

    /// Drena los flancos generados (llamar antes a `ear_level(now)`).
    pub fn drain_edges(&mut self) -> Vec<AudioEvent> {
        std::mem::take(&mut self.edges)
    }

    fn set_level(&mut self, e: u64, level: bool) {
        if self.level != level {
            self.level = level;
            self.edges.push(AudioEvent { tstate: e, level });
        }
    }

    fn toggle(&mut self, e: u64) {
        let l = !self.level;
        self.set_level(e, l);
    }

    fn stop_at(&mut self, e: u64) {
        if self.open_pulse {
            self.toggle(e);
            self.open_pulse = false;
        }
        self.playing = false;
        self.phase = None;
        tracing::debug!(target: "zx48::tape", tstate = e, event = "TAPE_STOPPED", block = self.block, end = self.at_end());
    }

    /// Avanza por bloques de control hasta el siguiente que genere señal o detenga la cinta.
    /// `e` es el T-state en que termina lo anterior.
    fn start_next(&mut self, e: u64) {
        for _ in 0..MAX_CONTROL_STEPS {
            let Some(blocks) = self.blocks.as_ref() else {
                return self.stop_at(e);
            };
            let Some(b) = blocks.get(self.block) else {
                return self.stop_at(e);
            };
            let action = match b {
                SignalBlock::Pause(0) => Action::Stop,
                SignalBlock::Pause(n) => Action::Pause(*n),
                SignalBlock::Nop => Action::Next,
                SignalBlock::SetLevel(l) => Action::SetLevel(*l),
                SignalBlock::LoopStart(n) => Action::LoopStart(*n),
                SignalBlock::LoopEnd => Action::LoopEnd,
                SignalBlock::Jump(d) => Action::Jump(*d),
                data => match data.render() {
                    Some(r) => Action::Load(r),
                    None => Action::Next,
                },
            };
            match action {
                Action::Load(r) if r.pulses.is_empty() => {
                    // Sin pulsos: solo la pausa (o nada).
                    if r.pause > 0 {
                        return self.begin_pause(e, r.pause);
                    }
                    self.block += 1;
                }
                Action::Load(r) => {
                    if let Some(l) = r.start_level {
                        // El primer flanco debe dejar la señal en `l`.
                        self.set_level(e, !l);
                    }
                    tracing::debug!(target: "zx48::tape", tstate = e, event = "BLOCK_START", block = self.block, pulses = r.pulses.len(), pause = r.pause);
                    self.pulses = r.pulses;
                    self.idx = 0;
                    self.cur_pause = r.pause;
                    self.phase = Some(Phase::Pulses);
                    self.next_edge = e;
                    return;
                }
                Action::Pause(n) => return self.begin_pause(e, n),
                Action::Stop => {
                    self.block += 1;
                    return self.stop_at(e);
                }
                Action::SetLevel(l) => {
                    self.set_level(e, l);
                    self.open_pulse = false;
                    self.block += 1;
                }
                Action::Next => self.block += 1,
                Action::LoopStart(n) => {
                    self.loops.push((self.block + 1, n.max(1) as u32));
                    self.block += 1;
                }
                Action::LoopEnd => match self.loops.last_mut() {
                    Some(top) => {
                        top.1 -= 1;
                        if top.1 > 0 {
                            self.block = top.0;
                        } else {
                            self.loops.pop();
                            self.block += 1;
                        }
                    }
                    None => self.block += 1,
                },
                Action::Jump(d) => {
                    let t = self.block as i64 + d as i64;
                    if t < 0 || t as usize >= blocks.len() {
                        self.block = blocks.len();
                    } else {
                        self.block = t as usize;
                    }
                }
            }
        }
        self.stop_at(e);
    }

    fn begin_pause(&mut self, e: u64, n: u64) {
        if self.open_pulse {
            self.toggle(e);
            self.open_pulse = false;
        }
        self.phase = Some(Phase::Pause);
        self.next_edge = e + n;
    }

    /// Nivel de EAR en el T-state maestro `now` (procesa todos los flancos pendientes).
    pub fn ear_level(&mut self, now: u64) -> bool {
        while self.playing && now >= self.next_edge {
            let e = self.next_edge;
            match self.phase {
                Some(Phase::Pulses) => {
                    if let Some(&d) = self.pulses.get(self.idx) {
                        self.toggle(e);
                        self.idx += 1;
                        self.next_edge = e + d as u64;
                        self.open_pulse = true;
                    } else if self.cur_pause > 0 {
                        self.toggle(e);
                        self.open_pulse = false;
                        self.phase = Some(Phase::Pause);
                        self.next_edge = e + self.cur_pause;
                    } else {
                        // Sin pausa: el flanco inicial del siguiente bloque cierra este pulso.
                        self.block += 1;
                        self.start_next(e);
                    }
                }
                Some(Phase::Pause) => {
                    self.block += 1;
                    self.start_next(e);
                }
                None => break,
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::tap::TapeHeaderKind;

    fn edges(p: &mut TapePlayer, from: u64, to: u64) -> Vec<u64> {
        let mut v = Vec::new();
        let mut last = p.ear_level(from);
        for t in from + 1..=to {
            let l = p.ear_level(t);
            if l != last {
                v.push(t);
                last = l;
            }
        }
        v
    }

    fn all_edges(blocks: Vec<SignalBlock>, until: u64) -> Vec<(u64, bool)> {
        let mut p = TapePlayer::new();
        p.insert(blocks);
        p.play(0);
        p.ear_level(until);
        p.drain_edges()
            .into_iter()
            .map(|e| (e.tstate, e.level))
            .collect()
    }

    #[test]
    fn pulse_counts_and_lengths() {
        let header = TapeBlock::header(TapeHeaderKind::Code, "x", 1, 0, 0);
        let p = block_pulses(&header);
        assert_eq!(p.len(), 8063 + 2 + 19 * 16);
        assert_eq!(&p[8063..8065], &[667, 735]);
        let data = TapeBlock::with_checksum(0xFF, &[0x80]);
        let p = block_pulses(&data);
        assert_eq!(p.len(), 3223 + 2 + 3 * 16);
        assert!(p[3225..3225 + 16].iter().all(|&d| d == 1710));
        assert_eq!(&p[3225 + 16..3225 + 18], &[1710, 1710]);
        assert!(p[3225 + 18..3225 + 32].iter().all(|&d| d == 855));
        assert!(block_pulses(&TapeBlock(vec![])).is_empty());
    }

    #[test]
    fn edges_follow_pulse_durations() {
        let mut p = TapePlayer::new();
        p.insert(Tape::from_blocks(vec![TapeBlock::with_checksum(0xFF, &[])]));
        p.play(100);
        assert!(!p.ear_level(99));
        let e = edges(&mut p, 99, 100 + 2168 * 3 + 10);
        assert_eq!(e, vec![100, 100 + 2168, 100 + 2 * 2168, 100 + 3 * 2168]);
    }

    #[test]
    fn plays_blocks_in_sequence_with_pause_then_stops() {
        let mut p = TapePlayer::new();
        p.insert(Tape::from_blocks(vec![
            TapeBlock::with_checksum(0xFF, &[]),
            TapeBlock::with_checksum(0xFF, &[]),
        ]));
        p.play(0);
        let one: u64 = 3223 * 2168 + 667 + 735 + 32 * 1710;
        p.ear_level(one + 1);
        assert_eq!(p.block_index(), 0);
        p.ear_level(one + PAUSE_TSTATES + 1);
        assert_eq!(p.block_index(), 1);
        assert!(p.is_playing());
        p.ear_level(2 * one + PAUSE_TSTATES * 3);
        assert!(!p.is_playing());
        assert!(p.at_end());
    }

    #[test]
    fn edge_log_records_exact_edge_times() {
        let mut p = TapePlayer::new();
        p.insert(Tape::from_blocks(vec![TapeBlock::with_checksum(
            0xFF,
            &[1],
        )]));
        p.play(50);
        p.ear_level(50 + 2168 * 2 + 1);
        let e = p.drain_edges();
        assert_eq!(
            e.iter().map(|x| (x.tstate, x.level)).collect::<Vec<_>>(),
            vec![(50, true), (50 + 2168, false), (50 + 4336, true)]
        );
        assert!(p.drain_edges().is_empty());
    }

    #[test]
    fn stop_and_rewind() {
        let mut p = TapePlayer::new();
        p.insert(Tape::from_blocks(vec![TapeBlock::with_checksum(
            0xFF,
            &[1],
        )]));
        p.play(0);
        p.ear_level(10_000);
        p.stop();
        let l = p.ear_level(1_000_000);
        assert_eq!(l, p.ear_level(2_000_000));
        p.rewind();
        assert_eq!(p.block_index(), 0);
        assert!(!p.is_playing());
    }

    #[test]
    fn composed_pure_blocks_equal_standard_block() {
        // 0x12 (piloto) + 0x13 (sync) + 0x14 (datos) deben generar exactamente la misma señal
        // que un bloque estándar: sin flancos espurios entre bloques.
        let data = vec![0xFF, 0x12, 0x34, 0xA5, 0xFF ^ 0x12 ^ 0x34 ^ 0xA5];
        let std = all_edges(vec![SignalBlock::standard(data.clone(), 3500)], 10_000_000);
        let composed = all_edges(
            vec![
                SignalBlock::Tone {
                    len: 2168,
                    count: 3223,
                },
                SignalBlock::Pulses(vec![667, 735]),
                SignalBlock::Data {
                    pilot: None,
                    sync: None,
                    zero: 855,
                    one: 1710,
                    used_bits: 8,
                    data,
                    pause: 3500,
                },
            ],
            10_000_000,
        );
        assert_eq!(std, composed);
        assert!(std.len() > 3223);
    }

    #[test]
    fn used_bits_truncates_last_byte() {
        let r = SignalBlock::Data {
            pilot: None,
            sync: None,
            zero: 10,
            one: 20,
            used_bits: 3,
            data: vec![0xFF, 0b1010_0000],
            pause: 0,
        }
        .render()
        .unwrap();
        // 8 bits del primer byte + 3 del segundo (1,0,1), cada bit = 2 pulsos.
        assert_eq!(r.pulses.len(), (8 + 3) * 2);
        assert_eq!(&r.pulses[16..], &[20, 20, 10, 10, 20, 20]);
    }

    #[test]
    fn turbo_timings_are_honoured() {
        let b = SignalBlock::Data {
            pilot: Some((1000, 4)),
            sync: Some((300, 400)),
            zero: 500,
            one: 900,
            used_bits: 8,
            data: vec![0b1000_0000],
            pause: 0,
        };
        let e = all_edges(vec![b], 100_000);
        let times: Vec<u64> = e.iter().map(|x| x.0).collect();
        let mut durations = vec![1000, 1000, 1000, 1000, 300, 400, 900, 900];
        durations.extend([500; 14]);
        let mut expect = vec![0u64];
        for d in durations {
            expect.push(expect.last().unwrap() + d);
        }
        // El último elemento es el flanco que cierra el último pulso al terminar la cinta.
        assert_eq!(times, expect);
    }

    #[test]
    fn direct_recording_merges_runs_and_sets_start_level() {
        // Muestras de 100 T: 1 1 1 0 0 1 0 0 | 1  (used_bits = 1 en el segundo byte)
        let b = SignalBlock::Direct {
            tstates: 100,
            used_bits: 1,
            data: vec![0b1110_0100, 0b1000_0000],
            pause: 0,
        };
        let r = b.render().unwrap();
        assert_eq!(r.start_level, Some(true));
        assert_eq!(r.pulses, vec![300, 200, 100, 200, 100]);
        let e = all_edges(vec![b], 10_000);
        assert_eq!(e[0], (0, true)); // empieza en alto
    }

    #[test]
    fn pause_blocks_and_stop() {
        let t = |v| SignalBlock::Tone { len: 100, count: v };
        let mut p = TapePlayer::new();
        p.insert(vec![
            t(2),
            SignalBlock::Pause(1000),
            t(2),
            SignalBlock::Pause(0),
            t(2),
        ]);
        p.play(0);
        p.ear_level(150);
        assert_eq!(p.block_index(), 0);
        // tono (2 pulsos: 0..200) + flanco de cierre + pausa de 1000
        p.ear_level(200 + 999);
        assert_eq!(p.block_index(), 1);
        p.ear_level(200 + 1000);
        assert_eq!(p.block_index(), 2);
        p.ear_level(10_000);
        // Se detiene en Pause(0) y el siguiente bloque queda pendiente.
        assert!(!p.is_playing());
        assert_eq!(p.block_index(), 4);
        // Reanudar continúa por el bloque siguiente.
        p.play(20_000);
        assert!(p.is_playing());
        p.ear_level(20_000 + 300);
        assert!(!p.is_playing());
        assert!(p.at_end());
    }

    #[test]
    fn loops_repeat_their_body() {
        let e = all_edges(
            vec![
                SignalBlock::LoopStart(3),
                SignalBlock::Pulses(vec![10, 20]),
                SignalBlock::LoopEnd,
                SignalBlock::Pause(1),
            ],
            1000,
        );
        let times: Vec<u64> = e.iter().map(|x| x.0).collect();
        // 3 × (10 + 20) = 90 T de pulsos y un flanco de cierre en 90 al llegar la pausa.
        assert_eq!(times, vec![0, 10, 30, 40, 60, 70, 90]);
    }

    #[test]
    fn jump_and_bad_jump() {
        // Salto +2: se salta el bloque intermedio.
        let e = all_edges(
            vec![
                SignalBlock::Jump(2),
                SignalBlock::Pulses(vec![999]),
                SignalBlock::Pulses(vec![5]),
            ],
            100,
        );
        // Flanco inicial del pulso de 5 T y flanco de cierre al terminar la cinta.
        assert_eq!(e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 5]);
        // Salto fuera de rango: termina sin colgarse.
        let mut p = TapePlayer::new();
        p.insert(vec![SignalBlock::Jump(-5)]);
        p.play(0);
        assert!(!p.is_playing());
        // Bucle infinito sin señal: se detiene en lugar de colgarse.
        let mut p = TapePlayer::new();
        p.insert(vec![SignalBlock::Jump(0)]);
        p.play(0);
        assert!(!p.is_playing());
    }

    #[test]
    fn set_level_block() {
        let e = all_edges(
            vec![SignalBlock::SetLevel(true), SignalBlock::Pause(10)],
            100,
        );
        assert_eq!(e[0], (0, true));
    }
}
