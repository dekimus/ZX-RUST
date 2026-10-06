# MEMORY.md — Persistent Context for the ZX Spectrum 48K Rust Emulator

## Mission

We are building a **Sinclair ZX Spectrum 48K emulator in Rust**, initially targeting the original 16K/48K architecture in a PAL-compatible timing profile.

Primary goal: **hardware-faithful, deterministic emulation**, not merely enough compatibility to run BASIC.

---

## Non-negotiable hardware facts

```text
CPU                 Zilog Z80A
CPU clock            3.500 MHz nominal
ROM                  16 KiB
RAM                  48 KiB
CPU address space    64 KiB

ROM                  0x0000..0x3FFF
RAM                  0x4000..0xFFFF

Display bitmap       0x4000..0x57FF  (6144 bytes)
Attributes            0x5800..0x5AFF  (768 bytes)
Display total         6912 bytes

Resolution             256×192
Text grid               32×24
Keyboard                8×5 matrix
ULA I/O                 port 0xFE (partially decoded)
```

There is **no memory banking** in the 48K model and **no AY sound chip**.

---

## Timing constants

```text
T-states/frame       69888
scanlines/frame      312
T-states/scanline    224
nominal frame rate   3500000 / 69888 ≈ 50.08 Hz

before display       64 scanlines = 14336 T
display              192 scanlines
after display        56 scanlines

per scanline:
  128 T display
   24 T right border
   48 T horizontal retrace
   24 T left border
```

The emulator must use a **master T-state counter**, never wall-clock time as the source of hardware state.

---

## ULA / port FE

### OUT

```text
bit 0..2 = border color
bit 3    = MIC
bit 4    = EAR output / internal speaker
bit 5..7 = unused
```

### IN

Port `0xFE` reads the keyboard according to the high address byte.

Rows:

```text
0xFEFE: SHIFT, Z, X, C, V
0xFDFE: A, S, D, F, G
0xFBFE: Q, W, E, R, T
0xF7FE: 1, 2, 3, 4, 5
0xEFFE: 0, 9, 8, 7, 6
0xDFFE: P, O, I, U, Y
0xBFFE: ENTER, L, K, J, H
0x7FFE: SPACE, SYMBOL SHIFT, M, N, B
```

Keyboard bits are active-low.

Bit 6 of IN `0xFE` is EAR input.

Multiple selected keyboard rows combine electrically; do not model the keyboard as an ASCII input device.

---

## Video memory

Bitmap address formula:

```text
addr =
    0x4000
    | ((y & 0xC0) << 5)
    | ((y & 0x07) << 8)
    | ((y & 0x38) << 2)
    | (x >> 3)
```

Expected boundary checks:

```text
(0,0)     -> 0x4000
(0,1)     -> 0x4100
(0,7)     -> 0x4700
(0,8)     -> 0x4020
(0,64)    -> 0x4800
(0,128)   -> 0x5000
(248,191) -> 0x57FF
```

Attribute address:

```text
0x5800 + (y >> 3) * 32 + (x >> 3)
```

Attribute bits:

```text
7     FLASH
6     BRIGHT
5..3  PAPER
2..0  INK
```

Colors:

```text
0 black
1 blue
2 red
3 magenta
4 green
5 cyan
6 yellow
7 white
```

---

## Contention

For 16K/48K:

```text
0x4000..0x7FFF = contended
0x8000..0xFFFF = uncontended
```

The ULA steals memory cycles while displaying the screen.

Canonical contention sequence:

```text
6,5,4,3,2,1,0,0
```

The first display contention window begins around T-state 14335/14336 after interrupt, depending on the exact timing convention/hardware phase.

**Do not model contention as a fixed extra cost per instruction.**

The cost depends on the exact T-state of the memory/I/O access.

---

## Z80 requirements

The CPU core must support:

- main opcodes;
- CB;
- ED;
- DD;
- FD;
- DD CB;
- FD CB;
- undocumented instructions/flags where needed for compatibility;
- accurate flags;
- HALT;
- EI/DI semantics;
- IFF1/IFF2;
- IM0/IM1/IM2;
- interrupt acceptance timing.

The ULA `/INT` pulse on 48K is approximately 32 T-states.

IM1 enters the standard interrupt vector at `0x0038`.

Do not implement interrupt handling as a simple high-level function call.

---

## Floating bus

Undecoded I/O and video-related reads can expose floating-bus values.

Returning `0xFF` for every unknown port is not sufficiently accurate.

Implement floating bus only after ULA video timing is correct because the value is tied to the byte being placed on the video bus at that moment.

---

## Tape

### TAP

TAP structure:

```text
u16 little-endian block length
block bytes
```

Standard ROM header:

```text
flag  0x00
type  0 PROGRAM
      1 number array
      2 character array
      3 CODE
name  10 bytes
length 2 bytes
param1 2 bytes
param2 2 bytes
checksum XOR
```

Standard data block starts with flag `0xFF` and ends with XOR checksum.

`SCREEN$`:

```text
type       CODE
start      16384
length     6912
```

There should be two tape paths:

1. faithful real-time EAR signal;
2. optional fast-loader acceleration.

Do not conflate them.

---

## Snapshots

### SNA 48K

Contains CPU state + border + 49152 bytes of RAM.

RAM corresponds to:

```text
0x4000..0xFFFF
```

The 48K SNA format stores PC indirectly via the stack; loading requires the documented `RETN` continuation behavior.

### Z80

Support at least:

- v1 48K;
- v2/v3 48K.

Z80 v1 has a 30-byte base header and optional compressed 48K RAM.

Compression marker:

```text
ED ED count value
```

End marker in v1 compressed stream:

```text
00 ED ED 00
```

v2/v3 use an extended header and 16 KiB memory blocks.

Reject unsupported hardware modes instead of partially loading them.

---

## ROM

The standard 16 KiB 16/48K ROM reference:

```text
SHA1 = 5ea7c2b824672e914525d1d5c419d71b84a426a2
MD5  = 4c42a2f075212361c3117015b107ff68
```

The emulator should load the ROM from an external file and validate size/checksum.

Do not redistribute copyrighted ROM contents.

The ROM is an integration test for:

```text
Z80 + memory + ULA + keyboard + video + tape + sound
```

---

## Rust architecture

Preferred separation:

```text
cpu/
machine/
ula/
input/
audio/
tape/
snapshot/
rom/
frontend/
```

The CPU should interact with the Spectrum through a bus abstraction.

Core should not depend on SDL/wgpu/winit.

Recommended conceptual state:

```rust
Spectrum48 {
    cpu,
    ula,
    memory,
    keyboard,
    tape,
    beeper,
    tstate,
}
```

The exact Rust representation can differ.

---

## Determinism

The emulator must be deterministic.

Wall-clock time belongs only to the frontend.

For identical:

```text
ROM
initial RAM
CPU state
ULA state
keyboard state
tape state
```

the sequence of machine events must be identical.

---

## Implementation priority

```text
1. Z80 correctness
2. memory map
3. ROM loading
4. ULA timing / frame clock
5. video
6. interrupts
7. keyboard + FE
8. beeper
9. contention
10. floating bus
11. TAP + ROM tape loader
12. SNA
13. Z80 snapshots
14. TZX/custom tape signals
15. frontend/debugger
```

Do not implement 128K features until the 48K model is correct.

---

## Testing priorities

Minimum tests:

- Z80 instruction/flag suite.
- Memory boundary tests.
- Display address formula tests.
- 69888 T/frame test.
- 224 T/scanline test.
- interrupt timing.
- border raster changes.
- contention.
- keyboard matrix.
- EAR input.
- beeper transitions.
- TAP parsing/checksum.
- SNA load/save.
- Z80 v1/v2/v3 load.
- ROM boot smoke test.
- headless deterministic execution.

A real ROM boot is more valuable than a custom BASIC implementation.

---

## Important anti-patterns

Never:

```text
sleep 20ms -> interrupt
```

Never:

```text
all accesses below 0x8000 cost +N cycles
```

Never:

```text
screen[y * 256 + x] directly from RAM
```

Never:

```text
unknown port read = 0xFF
```

Never:

```text
TAP = copy block bytes directly into RAM
```

when claiming faithful emulation.

---

## Reference sources

- https://www.worldofspectrum.net/faq/reference/48kreference.htm
- https://worldofspectrum.org/faq/reference/z80reference.htm
- https://www.worldofspectrum.net/faq/reference/z80format.htm
- https://worldofspectrum.net/zx-modules/fileformats/snaformat.html
- https://worldofspectrum.net/zx-modules/fileformats/tapformat.html
- https://www.worldofspectrum.net/faq/reference/formats.htm
- https://spectrumforeveryone.com/wp-content/uploads/2017/08/ZX-Spectrum-Service-Manual.pdf
- https://sinclair.wiki.zxnet.co.uk/wiki/ROM_images
- https://sinclair.wiki.zxnet.co.uk/wiki/Contended_memory

---

## Agent rule

When uncertain, prefer the documented physical/hardware behavior over a convenient abstraction.

If a timing value is disputed between sources, keep the uncertainty explicit in code/comments and create a targeted regression test rather than silently choosing a value.
