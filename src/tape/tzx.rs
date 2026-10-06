//! Parser de `.tzx` (v1.x), independiente de la emulación: produce bloques de señal.
//!
//! Soportado: 0x10 (datos estándar), 0x11 (turbo), 0x12 (tono), 0x13 (pulsos), 0x14 (datos puros),
//! 0x15 (grabación directa), 0x20 (pausa/stop), 0x21/0x22 (grupos), 0x23 (salto), 0x24/0x25 (bucle),
//! 0x2A (parar si 48K), 0x2B (nivel), 0x30-0x35 y 0x5A (informativos).
//! Explícitamente NO soportado (error, no se ignora en silencio): 0x18 CSW, 0x19 datos generalizados,
//! 0x26/0x27 llamadas, 0x28 selección, 0x2C-0x2F y bloques desconocidos < 0x30.
//! Bloques desconocidos >= 0x30 se saltan con su longitud de 4 bytes (regla de extensión de la especificación).
//!
//! Incertidumbre: en grabación directa el nivel inicial se respeta; en el resto solo importan los flancos.

use super::signal::{Playable, SignalBlock, TSTATES_PER_MS};
use crate::error::EmulatorError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tzx {
    pub version: (u8, u8),
    pub blocks: Vec<SignalBlock>,
}

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], EmulatorError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.d.len())
            .ok_or_else(|| {
                EmulatorError::InvalidTape(format!("bloque TZX truncado en el byte {}", self.pos))
            })?;
        let s = &self.d[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, EmulatorError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, EmulatorError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u24(&mut self) -> Result<usize, EmulatorError> {
        let b = self.take(3)?;
        Ok(b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16)
    }
    fn u32(&mut self) -> Result<usize, EmulatorError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }
}

fn ms(v: u16) -> u64 {
    v as u64 * TSTATES_PER_MS
}

impl Tzx {
    pub fn parse(data: &[u8]) -> Result<Self, EmulatorError> {
        if data.len() < 10 || &data[..7] != b"ZXTape!" || data[7] != 0x1A {
            return Err(EmulatorError::InvalidTape(
                "cabecera TZX inválida (se esperaba 'ZXTape!' + 0x1A)".into(),
            ));
        }
        let version = (data[8], data[9]);
        if version.0 != 1 {
            return Err(EmulatorError::UnsupportedTape(format!(
                "TZX versión {}.{}",
                version.0, version.1
            )));
        }
        let mut r = Reader { d: data, pos: 10 };
        let mut blocks = Vec::new();
        while r.pos < data.len() {
            let id = r.u8()?;
            blocks.push(Self::block(&mut r, id)?);
        }
        Ok(Self { version, blocks })
    }

    fn block(r: &mut Reader, id: u8) -> Result<SignalBlock, EmulatorError> {
        Ok(match id {
            0x10 => {
                let pause = r.u16()?;
                let len = r.u16()? as usize;
                SignalBlock::standard(r.take(len)?.to_vec(), ms(pause))
            }
            0x11 => {
                let pilot = r.u16()? as u32;
                let s1 = r.u16()? as u32;
                let s2 = r.u16()? as u32;
                let zero = r.u16()? as u32;
                let one = r.u16()? as u32;
                let count = r.u16()? as u32;
                let used_bits = r.u8()?;
                let pause = r.u16()?;
                let len = r.u24()?;
                SignalBlock::Data {
                    pilot: Some((pilot, count)),
                    sync: Some((s1, s2)),
                    zero,
                    one,
                    used_bits,
                    data: r.take(len)?.to_vec(),
                    pause: ms(pause),
                }
            }
            0x12 => SignalBlock::Tone {
                len: r.u16()? as u32,
                count: r.u16()? as u32,
            },
            0x13 => {
                let n = r.u8()? as usize;
                let mut p = Vec::with_capacity(n);
                for _ in 0..n {
                    p.push(r.u16()? as u32);
                }
                SignalBlock::Pulses(p)
            }
            0x14 => {
                let zero = r.u16()? as u32;
                let one = r.u16()? as u32;
                let used_bits = r.u8()?;
                let pause = r.u16()?;
                let len = r.u24()?;
                SignalBlock::Data {
                    pilot: None,
                    sync: None,
                    zero,
                    one,
                    used_bits,
                    data: r.take(len)?.to_vec(),
                    pause: ms(pause),
                }
            }
            0x15 => {
                let tstates = r.u16()? as u32;
                let pause = r.u16()?;
                let used_bits = r.u8()?;
                let len = r.u24()?;
                SignalBlock::Direct {
                    tstates,
                    used_bits,
                    data: r.take(len)?.to_vec(),
                    pause: ms(pause),
                }
            }
            0x20 => SignalBlock::Pause(ms(r.u16()?)),
            0x21 => {
                let n = r.u8()? as usize;
                r.take(n)?;
                SignalBlock::Nop
            }
            0x22 => SignalBlock::Nop,
            0x23 => SignalBlock::Jump(r.u16()? as i16 as i32),
            0x24 => SignalBlock::LoopStart(r.u16()?),
            0x25 => SignalBlock::LoopEnd,
            0x2A => {
                let n = r.u32()?;
                r.take(n)?;
                // Esta máquina es un 48K: el bloque ordena parar la cinta.
                SignalBlock::Pause(0)
            }
            0x2B => {
                r.u32()?;
                SignalBlock::SetLevel(r.u8()? != 0)
            }
            0x30 => {
                let n = r.u8()? as usize;
                r.take(n)?;
                SignalBlock::Nop
            }
            0x31 => {
                r.u8()?;
                let n = r.u8()? as usize;
                r.take(n)?;
                SignalBlock::Nop
            }
            0x32 => {
                let n = r.u16()? as usize;
                r.take(n)?;
                SignalBlock::Nop
            }
            0x33 => {
                let n = r.u8()? as usize;
                r.take(n * 3)?;
                SignalBlock::Nop
            }
            0x35 => {
                r.take(10)?;
                let n = r.u32()?;
                r.take(n)?;
                SignalBlock::Nop
            }
            0x5A => {
                r.take(9)?;
                SignalBlock::Nop
            }
            0x18 | 0x19 | 0x26 | 0x27 | 0x28 | 0x2C..=0x2F => {
                return Err(EmulatorError::UnsupportedTape(format!(
                    "bloque TZX {id:#04X}"
                )));
            }
            id if id >= 0x30 => {
                let n = r.u32()?;
                r.take(n)?;
                SignalBlock::Nop
            }
            id => {
                return Err(EmulatorError::InvalidTape(format!(
                    "bloque TZX desconocido {id:#04X}"
                )));
            }
        })
    }
}

impl From<Tzx> for Playable {
    fn from(t: Tzx) -> Self {
        Playable(t.blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::signal::{PAUSE_TSTATES, TapePlayer};
    use crate::tape::tap::{Tape, TapeBlock};

    fn tzx(blocks: &[&[u8]]) -> Vec<u8> {
        let mut v = b"ZXTape!\x1A\x01\x14".to_vec();
        for b in blocks {
            v.extend_from_slice(b);
        }
        v
    }

    fn b10(pause_ms: u16, data: &[u8]) -> Vec<u8> {
        let mut v = vec![0x10];
        v.extend_from_slice(&pause_ms.to_le_bytes());
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    fn edges(p: impl Into<Playable>, until: u64) -> Vec<(u64, bool)> {
        let mut pl = TapePlayer::new();
        pl.insert(p);
        pl.play(0);
        pl.ear_level(until);
        pl.drain_edges()
            .into_iter()
            .map(|e| (e.tstate, e.level))
            .collect()
    }

    #[test]
    fn block_10_is_identical_to_tap_signal() {
        let payload = TapeBlock::with_checksum(0xFF, &[1, 2, 3, 4]);
        let t = Tzx::parse(&tzx(&[&b10(1000, &payload.0)])).unwrap();
        assert_eq!(t.version, (1, 20));
        let tap = Tape::from_blocks(vec![payload]);
        assert_eq!(edges(t, 30_000_000), edges(tap, 30_000_000));
    }

    #[test]
    fn turbo_block_0x11_fields() {
        let mut b = vec![0x11];
        for w in [1000u16, 300, 400, 500, 900, 4] {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b.push(8); // used bits
        b.extend_from_slice(&0u16.to_le_bytes()); // pause
        b.extend_from_slice(&[1, 0, 0]); // len
        b.push(0b1000_0000);
        let t = Tzx::parse(&tzx(&[&b])).unwrap();
        let e = edges(t, 100_000);
        let times: Vec<u64> = e.iter().map(|x| x.0).collect();
        assert_eq!(times[..6], [0, 1000, 2000, 3000, 4000, 4300]);
        assert_eq!(times[6], 4700);
        assert_eq!(times[7], 5600); // bit 1: 900 + 900
        assert_eq!(times[8], 6500);
        assert_eq!(times[9], 7000); // bits 0: 500 cada pulso
    }

    #[test]
    fn tone_pulses_pure_data_compose() {
        let mut v = vec![0x12];
        v.extend_from_slice(&2168u16.to_le_bytes());
        v.extend_from_slice(&5u16.to_le_bytes());
        v.extend_from_slice(&[0x13, 2, 0x9B, 0x02, 0xDF, 0x02]); // 667, 735
        let mut d = vec![0x14];
        d.extend_from_slice(&855u16.to_le_bytes());
        d.extend_from_slice(&1710u16.to_le_bytes());
        d.push(8);
        d.extend_from_slice(&1000u16.to_le_bytes());
        d.extend_from_slice(&[1, 0, 0, 0xA5]);
        let t = Tzx::parse(&tzx(&[&v[..5], &v[5..], &d])).unwrap();
        assert_eq!(t.blocks.len(), 3);
        assert!(matches!(
            t.blocks[0],
            SignalBlock::Tone {
                len: 2168,
                count: 5
            }
        ));
        assert_eq!(t.blocks[1], SignalBlock::Pulses(vec![667, 735]));
    }

    #[test]
    fn informational_blocks_are_nops_keeping_numbering() {
        let t = Tzx::parse(&tzx(&[
            &[0x21, 3, b'a', b'b', b'c'],
            &[0x22],
            &[0x30, 2, b'h', b'i'],
            &[0x31, 5, 2, b'o', b'k'],
            &[0x32, 3, 0, 0, 0, 0],
            &[0x33, 1, 0, 0, 0],
            &[0x5A, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            &[0x20, 0xE8, 0x03],
        ]))
        .unwrap();
        assert_eq!(t.blocks.len(), 8);
        assert!(t.blocks[..7].iter().all(|b| *b == SignalBlock::Nop));
        assert_eq!(t.blocks[7], SignalBlock::Pause(3_500_000));
    }

    #[test]
    fn stop_if_48k_and_set_level_and_jump_and_loop() {
        let t = Tzx::parse(&tzx(&[
            &[0x2A, 0, 0, 0, 0],
            &[0x2B, 1, 0, 0, 0, 1],
            &[0x23, 0xFE, 0xFF],
            &[0x24, 3, 0],
            &[0x25],
        ]))
        .unwrap();
        assert_eq!(
            t.blocks,
            vec![
                SignalBlock::Pause(0),
                SignalBlock::SetLevel(true),
                SignalBlock::Jump(-2),
                SignalBlock::LoopStart(3),
                SignalBlock::LoopEnd
            ]
        );
    }

    #[test]
    fn unknown_extension_blocks_are_skipped_by_length() {
        let t = Tzx::parse(&tzx(&[&[0x40, 3, 0, 0, 0, 9, 9, 9], &[0x20, 1, 0]])).unwrap();
        assert_eq!(t.blocks, vec![SignalBlock::Nop, SignalBlock::Pause(3500)]);
    }

    #[test]
    fn malformed_and_unsupported_files_are_rejected() {
        assert!(Tzx::parse(b"ZXTape").is_err());
        assert!(Tzx::parse(b"XXTape!\x1A\x01\x14").is_err());
        assert!(Tzx::parse(b"ZXTape!\x1A\x02\x00").is_err());
        // truncados
        assert!(Tzx::parse(&tzx(&[&[0x10, 0, 0, 5, 0, 1]])).is_err());
        assert!(Tzx::parse(&tzx(&[&[0x12, 1]])).is_err());
        assert!(Tzx::parse(&tzx(&[&[0x21, 5, b'a']])).is_err());
        // no soportados / desconocidos
        for id in [0x18u8, 0x19, 0x26, 0x27, 0x28, 0x2C] {
            assert!(
                matches!(
                    Tzx::parse(&tzx(&[&[id, 0, 0, 0, 0]])),
                    Err(EmulatorError::UnsupportedTape(_))
                ),
                "{id:#X}"
            );
        }
        assert!(matches!(
            Tzx::parse(&tzx(&[&[0x16, 0]])),
            Err(EmulatorError::InvalidTape(_))
        ));
    }

    #[test]
    fn empty_tzx_is_valid_and_silent() {
        let t = Tzx::parse(&tzx(&[])).unwrap();
        assert!(t.blocks.is_empty());
        assert!(edges(t, 1000).is_empty());
    }

    #[test]
    fn pause_zero_in_data_block_means_no_pause() {
        // Dos bloques 0x10 seguidos sin pausa: sin flanco extra entre ellos ni silencio.
        let a = TapeBlock::with_checksum(0xFF, &[]);
        let t = Tzx::parse(&tzx(&[&b10(0, &a.0), &b10(0, &a.0)])).unwrap();
        let e = edges(t, 100_000_000);
        // El segundo bloque empieza exactamente cuando termina el último pulso del primero.
        let first_len: u64 = 3223 * 2168 + 667 + 735 + 32 * 1710;
        assert!(e.iter().any(|x| x.0 == first_len));
        let _ = PAUSE_TSTATES;
    }
}
