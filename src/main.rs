// Cat Tetris in Rust (macroquad).
// - Cleared rows are eaten by running cats (with crunch sounds).
// - Every so often a CAT piece drops: a single block that explodes on landing
//   and wipes out a square area around it.
// - All graphics are drawn with code and all sounds are synthesized at startup
//   (meows use a formant voice model), so there are no asset files.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use macroquad::audio::{self, play_sound, play_sound_once, stop_sound, PlaySoundParams, Sound};
use macroquad::prelude::*;
use std::f32::consts::{FRAC_PI_2, PI};

const W: usize = 10;
const H: usize = 20;
const CELL: f32 = 30.0;
const BX: f32 = 40.0; // board origin
const BY: f32 = 50.0;
const PANEL_X: f32 = 375.0;
const WIN_W: f32 = 600.0;
const WIN_H: f32 = 700.0;

// ---- tuning ----
const CAT_ONE_IN: u32 = 10; // a cat piece appears on average once per 10 pieces
const BOMB_R: i32 = 2; // blast radius in cells: 2 = 5x5 area, 1 = 3x3 area
const CLEAR_DUR: f32 = 0.9; // seconds the cats take to eat the rows
const BOOM_DUR: f32 = 0.5;
const FLOAT_DUR: f32 = 1.3;
const LOCK_FLASH: f32 = 0.18;

// ---------------------------------------------------------------- pieces ----

// 0=I 1=O 2=T 3=S 4=Z 5=J 6=L 7=CAT (single exploding block)
const CAT: usize = 7;
const SIZES: [usize; 8] = [4, 2, 3, 3, 3, 3, 3, 1];
const SHAPES: [[[u8; 4]; 4]; 8] = [
    [[0, 0, 0, 0], [1, 1, 1, 1], [0, 0, 0, 0], [0, 0, 0, 0]],
    [[1, 1, 0, 0], [1, 1, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
    [[0, 1, 0, 0], [1, 1, 1, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
    [[0, 1, 1, 0], [1, 1, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
    [[1, 1, 0, 0], [0, 1, 1, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
    [[1, 0, 0, 0], [1, 1, 1, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
    [[0, 0, 1, 0], [1, 1, 1, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
    [[1, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]],
];

fn piece_color(kind: usize) -> Color {
    match kind {
        0 => Color::new(0.00, 0.85, 0.90, 1.0), // I cyan
        1 => Color::new(0.95, 0.85, 0.10, 1.0), // O yellow
        2 => Color::new(0.65, 0.20, 0.85, 1.0), // T purple
        3 => Color::new(0.20, 0.80, 0.25, 1.0), // S green
        4 => Color::new(0.90, 0.20, 0.20, 1.0), // Z red
        5 => Color::new(0.20, 0.35, 0.95, 1.0), // J blue
        7 => Color::new(0.75, 0.75, 0.80, 1.0), // cat gray
        _ => Color::new(0.97, 0.55, 0.10, 1.0), // L orange
    }
}

#[derive(Clone, Copy)]
struct Piece {
    kind: usize,
    n: usize,
    g: [[bool; 4]; 4],
    x: i32,
    y: i32,
}

impl Piece {
    fn new(kind: usize) -> Self {
        let n = SIZES[kind];
        let mut g = [[false; 4]; 4];
        for r in 0..4 {
            for c in 0..4 {
                g[r][c] = SHAPES[kind][r][c] == 1;
            }
        }
        Piece {
            kind,
            n,
            g,
            x: (W as i32 - n as i32) / 2,
            y: if kind == 0 { -1 } else { 0 },
        }
    }

    fn cells(&self) -> Vec<(i32, i32)> {
        let mut v = Vec::with_capacity(4);
        for r in 0..self.n {
            for c in 0..self.n {
                if self.g[r][c] {
                    v.push((self.x + c as i32, self.y + r as i32));
                }
            }
        }
        v
    }

    fn rotated(&self, cw: bool) -> Piece {
        let mut p = *self;
        let n = self.n;
        for r in 0..n {
            for c in 0..n {
                p.g[r][c] = if cw { self.g[n - 1 - c][r] } else { self.g[c][n - 1 - r] };
            }
        }
        p
    }
}

// ----------------------------------------------------------- audio: basics --

const SR: u32 = 44100;

fn sr() -> f32 {
    SR as f32
}

struct Rng(u32);

impl Rng {
    // uniform in [-1, 1)
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        ((self.0 >> 8) as f32 / 8_388_608.0) - 1.0
    }
}

fn add_at(buf: &mut Vec<f32>, start: usize, src: &[f32], gain: f32) {
    if buf.len() < start + src.len() {
        buf.resize(start + src.len(), 0.0);
    }
    for (i, s) in src.iter().enumerate() {
        buf[start + i] += s * gain;
    }
}

fn normalize(buf: &mut [f32], peak: f32) {
    let m = buf.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    if m > 1e-6 {
        let g = peak / m;
        for s in buf.iter_mut() {
            *s *= g;
        }
    }
}

// Small room reverb (parallel feedback combs). `wet` ~0.1 .. 0.4
fn reverb(buf: &mut Vec<f32>, wet: f32) {
    let dry = buf.clone();
    let tail = (0.4 * sr()) as usize;
    let len = dry.len() + tail;
    let mut out = vec![0.0f32; len];
    out[..dry.len()].copy_from_slice(&dry);
    for (ms, fb) in [(29.7f32, 0.64f32), (37.1, 0.60), (41.1, 0.57), (43.7, 0.54)] {
        let d = (ms / 1000.0 * sr()) as usize;
        let mut comb = vec![0.0f32; len];
        for i in 0..len {
            let x = if i < dry.len() { dry[i] } else { 0.0 };
            let f = if i >= d { comb[i - d] } else { 0.0 };
            comb[i] = x + fb * f;
            out[i] += (comb[i] - x) * wet * 0.5;
        }
    }
    *buf = out;
}

// Wrap the reverb tail around to the start so a looped track has no gap.
fn fold_loop(buf: &mut Vec<f32>, len: usize) {
    for i in len..buf.len() {
        let j = (i - len) % len;
        let v = buf[i];
        buf[j] += v;
    }
    buf.truncate(len);
}

fn to_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + samples.len() * 2);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 30000.0) as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

// ---------------------------------------------------------- audio: voices ----

// Soft sine blip with a pitch glide (UI sounds, thumps).
fn blip(f0: f32, f1: f32, dur: f32, vol: f32) -> Vec<f32> {
    let n = (dur * sr()) as usize;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / n as f32;
        let secs = i as f32 / sr();
        let f = f0 * (f1 / f0).powf(t);
        let env = (1.0 - (-secs / 0.002).exp()) * (-6.0 * t).exp() * (1.0 - t);
        out.push((2.0 * PI * phase).sin() * env * vol);
        phase += f / sr();
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
    out
}

// Decaying noise; `bright` = high-passed (clicks, hats, crunches), else dull (rumble).
fn noise_burst(dur: f32, vol: f32, decay: f32, bright: bool, seed: u32) -> Vec<f32> {
    let n = (dur * sr()) as usize;
    let mut rng = Rng(seed);
    let mut lp = 0.0f32;
    let mut prev = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let r = rng.next();
        let x = if bright {
            let d = r - prev;
            prev = r;
            d * 0.5
        } else {
            lp += 0.12 * (r - lp);
            lp * 3.0
        };
        let secs = i as f32 / sr();
        out.push(x * (-secs * decay).exp() * vol);
    }
    out
}

// Plucked / marimba-like note.
fn pluck(freq: f32, dur: f32, vol: f32) -> Vec<f32> {
    let n = (dur * sr()) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / sr();
        let tau = 2.0 * PI * freq * t;
        let env = (1.0 - (-t / 0.004).exp()) * (-t * 5.0).exp();
        let s = tau.sin()
            + 0.4 * (2.0 * tau).sin() * (-t * 9.0).exp()
            + 0.15 * (3.0 * tau).sin() * (-t * 14.0).exp();
        let rel = ((n - i) as f32 / (0.02 * sr())).min(1.0);
        out.push(s * env * rel * vol);
    }
    out
}

fn bass_note(freq: f32, dur: f32, vol: f32) -> Vec<f32> {
    let n = (dur * sr()) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / sr();
        let tau = 2.0 * PI * freq * t;
        let env = (1.0 - (-t / 0.006).exp()) * (-t * 3.5).exp();
        let rel = ((n - i) as f32 / (0.02 * sr())).min(1.0);
        out.push((tau.sin() + 0.45 * (2.0 * tau).sin()) * env * rel * vol);
    }
    out
}

fn kick(vol: f32) -> Vec<f32> {
    let n = (0.2 * sr()) as usize;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / sr();
        let f = 45.0 + 110.0 * (-t * 30.0).exp();
        out.push((2.0 * PI * phase).sin() * (-t * 16.0).exp() * vol);
        phase += f / sr();
    }
    out
}

fn snare(vol: f32) -> Vec<f32> {
    let mut out = noise_burst(0.15, vol, 22.0, true, 77);
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / sr();
        *s += (2.0 * PI * 185.0 * t).sin() * (-t * 30.0).exp() * vol * 0.5;
    }
    out
}

// ------------------------------------------------------- audio: cat voice ----

#[derive(Clone, Copy)]
struct Vow {
    f1: f32,
    f2: f32,
    f3: f32,
}

// Vowel targets (human-like values; scaled up for a cat-sized vocal tract).
const VM: Vow = Vow { f1: 300.0, f2: 1000.0, f3: 2400.0 };
const VI: Vow = Vow { f1: 350.0, f2: 2500.0, f3: 3200.0 };
const VE: Vow = Vow { f1: 550.0, f2: 2000.0, f3: 2900.0 };
const VA: Vow = Vow { f1: 850.0, f2: 1500.0, f3: 2800.0 };
const VO: Vow = Vow { f1: 480.0, f2: 850.0, f3: 2500.0 };
const VU: Vow = Vow { f1: 380.0, f2: 800.0, f3: 2400.0 };
const CAT_TRACT: f32 = 1.25;

#[derive(Clone, Copy)]
enum Voice {
    Miau, // "mi-au"
    Mew,  // short kitten "mew"
    Sad,  // long, falling "miaaow"
}

fn track(keys: &[(f32, f32)], t: f32) -> f32 {
    if t <= keys[0].0 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        if t <= w[1].0 {
            let u = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
            return w[0].1 + (w[1].1 - w[0].1) * u;
        }
    }
    keys[keys.len() - 1].1
}

fn vowel_at(keys: &[(f32, Vow, f32)], t: f32) -> (Vow, f32) {
    if t <= keys[0].0 {
        return (keys[0].1, keys[0].2);
    }
    for w in keys.windows(2) {
        if t <= w[1].0 {
            let u = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
            let a = w[0].1;
            let b = w[1].1;
            let v = Vow {
                f1: a.f1 + (b.f1 - a.f1) * u,
                f2: a.f2 + (b.f2 - a.f2) * u,
                f3: a.f3 + (b.f3 - a.f3) * u,
            };
            return (v, w[0].2 + (w[1].2 - w[0].2) * u);
        }
    }
    let l = keys[keys.len() - 1];
    (l.1, l.2)
}

fn glottal(p: f32) -> f32 {
    // Rosenberg-style glottal flow pulse
    if p < 0.45 {
        0.5 * (1.0 - (PI * p / 0.45).cos())
    } else if p < 0.62 {
        ((p - 0.45) / 0.17 * FRAC_PI_2).cos()
    } else {
        0.0
    }
}

// Source-filter meow: a glottal pulse train (with pitch contour, vibrato,
// jitter and breath noise) drives three moving formant resonators that glide
// m -> i -> a -> u, like a real "miau". `f0` is the average pitch in Hz.
fn meow_buf(f0: f32, dur: f32, vol: f32, voice: Voice, seed: u32) -> Vec<f32> {
    let (pitch, vow): (Vec<(f32, f32)>, Vec<(f32, Vow, f32)>) = match voice {
        Voice::Miau => (
            vec![(0.0, 0.85), (0.22, 1.18), (0.5, 1.0), (1.0, 0.62)],
            vec![
                (0.0, VM, 0.30),
                (0.10, VM, 0.55),
                (0.22, VI, 0.90),
                (0.40, VA, 1.00),
                (0.65, VA, 0.85),
                (0.85, VU, 0.50),
                (1.0, VU, 0.0),
            ],
        ),
        Voice::Mew => (
            vec![(0.0, 0.90), (0.35, 1.20), (1.0, 0.80)],
            vec![
                (0.0, VM, 0.30),
                (0.12, VM, 0.60),
                (0.30, VI, 0.95),
                (0.65, VE, 0.90),
                (0.90, VU, 0.40),
                (1.0, VU, 0.0),
            ],
        ),
        Voice::Sad => (
            vec![(0.0, 1.00), (0.18, 1.15), (0.5, 0.95), (1.0, 0.55)],
            vec![
                (0.0, VM, 0.30),
                (0.12, VM, 0.60),
                (0.25, VI, 0.80),
                (0.45, VA, 1.00),
                (0.80, VO, 0.80),
                (1.0, VO, 0.0),
            ],
        ),
    };

    let n = (dur * sr()).max(1.0) as usize;
    let mut rng = Rng(seed.wrapping_mul(2654435761).wrapping_add(12345));
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    let mut prev_flow = 0.0f32;
    let mut jit = 0.0f32;
    let mut y1 = [0.0f32; 3];
    let mut y2 = [0.0f32; 3];
    let weights = [1.0f32, 0.55, 0.30];
    let bws = [110.0f32, 160.0, 260.0];

    for i in 0..n {
        let t = i as f32 / n as f32;
        let secs = i as f32 / sr();

        // pitch: contour + vibrato + slow random jitter
        jit = 0.995 * jit + 0.004 * rng.next();
        let vib = 0.018 * (2.0 * PI * 5.5 * secs).sin() * (t * 4.0).min(1.0);
        let f = (f0 * track(&pitch, t) * (1.0 + vib + jit)).max(60.0);
        phase += f / sr();
        if phase >= 1.0 {
            phase -= 1.0;
        }

        // voice source: derivative of glottal flow + breath noise
        let flow = glottal(phase);
        let mut src = (flow - prev_flow) * sr() / f;
        prev_flow = flow;
        let breath = 0.35 * (1.0 - (t * PI).sin() * 0.7);
        src += rng.next() * breath * 0.3;

        // moving formants
        let (v, amp) = vowel_at(&vow, t);
        let fs = [v.f1 * CAT_TRACT, v.f2 * CAT_TRACT, v.f3 * CAT_TRACT];
        let mut s = 0.0f32;
        for k in 0..3 {
            let r = (-PI * bws[k] / sr()).exp();
            let c1 = 2.0 * r * (2.0 * PI * fs[k] / sr()).cos();
            let c2 = -r * r;
            let y = (1.0 - r) * src + c1 * y1[k] + c2 * y2[k];
            y2[k] = y1[k];
            y1[k] = y;
            s += weights[k] * y;
        }
        let attack = (secs / 0.015).min(1.0);
        out.push(s * amp * attack);
    }
    normalize(&mut out, vol);
    out
}

// ---------------------------------------------------- audio: sound design ----

struct SoundData {
    mv: Vec<f32>,
    rot: Vec<f32>,
    lock: Vec<f32>,
    drop: Vec<f32>,
    clear: Vec<f32>,
    tetris: Vec<f32>,
    level: Vec<f32>,
    over: Vec<f32>,
    mew: Vec<f32>,
    boom: Vec<f32>,
    music: Vec<f32>,
}

enum Note {
    Tone(f32, f32), // frequency, beats
    Meow(f32, f32), // average pitch, beats
    Rest(f32),
}

// Original bouncy tune: marimba-ish lead, bass, light drums, and real-sounding
// cats meowing "miau miau" in the melody.
fn build_music() -> Vec<f32> {
    use Note::*;
    const G2: f32 = 98.0;
    const A2: f32 = 110.0;
    const C3: f32 = 130.81;
    const D3: f32 = 146.83;
    const E3: f32 = 164.81;
    const F3: f32 = 174.61;
    const G3: f32 = 196.0;
    const C4: f32 = 261.63;
    const G4: f32 = 392.0;
    const A4: f32 = 440.0;
    const B4: f32 = 493.88;
    const C5: f32 = 523.25;
    const D5: f32 = 587.33;
    const E5: f32 = 659.25;
    const F5: f32 = 698.46;
    const G5: f32 = 783.99;
    const A5: f32 = 880.0;

    let q = 0.22f32; // seconds per beat
    let beats = 32usize;
    let len = (beats as f32 * q * sr()) as usize;
    let mut buf = vec![0.0f32; len];

    let melody = vec![
        // phrase 1
        Tone(E5, 0.5), Tone(G5, 0.5), Tone(E5, 0.5), Tone(C5, 0.5), Meow(660.0, 2.0),
        Tone(D5, 0.5), Tone(F5, 0.5), Tone(D5, 0.5), Tone(B4, 0.5), Meow(590.0, 2.0),
        // phrase 2
        Tone(C5, 0.5), Tone(E5, 0.5), Tone(G5, 0.5), Tone(E5, 0.5), Tone(A5, 1.0), Tone(G5, 1.0),
        Tone(E5, 0.5), Tone(D5, 0.5), Tone(C5, 0.5), Tone(D5, 0.5), Tone(C5, 1.5), Rest(0.5),
        // phrase 3
        Tone(A4, 0.5), Tone(C5, 0.5), Tone(E5, 0.5), Tone(C5, 0.5), Meow(520.0, 2.0),
        Tone(G4, 0.5), Tone(B4, 0.5), Tone(D5, 0.5), Tone(B4, 0.5), Meow(700.0, 2.0),
        // phrase 4
        Tone(E5, 0.5), Tone(G5, 0.5), Tone(A5, 0.5), Tone(G5, 0.5), Tone(E5, 1.0), Tone(C5, 1.0),
        Tone(D5, 0.5), Tone(E5, 0.5), Tone(D5, 0.5), Tone(B4, 0.5), Tone(C5, 1.5), Rest(0.5),
    ];

    let mut beat_pos = 0.0f32;
    let mut meow_idx = 0u32;
    for note in melody {
        let start = (beat_pos * q * sr()) as usize;
        match note {
            Tone(f, b) => {
                add_at(&mut buf, start, &pluck(f, b * q * 1.8, 0.30), 1.0);
                beat_pos += b;
            }
            Meow(f, b) => {
                meow_idx += 1;
                add_at(&mut buf, start, &meow_buf(f, b * q, 0.75, Voice::Miau, 40 + meow_idx), 1.0);
                beat_pos += b;
            }
            Rest(b) => beat_pos += b,
        }
    }

    // one bass note per beat
    let bass_notes = [
        C3, G3, C3, G3, G2, D3, G2, D3, C3, G3, C3, G3, G2, D3, C3, C3,
        A2, E3, A2, E3, G2, D3, G2, D3, F3, C4, F3, C4, G2, D3, C3, C3,
    ];
    let kick_s = kick(0.55);
    let snare_s = snare(0.18);
    let hat_s = noise_burst(0.04, 0.07, 100.0, true, 5);
    for (b, f) in bass_notes.iter().enumerate() {
        let start = (b as f32 * q * sr()) as usize;
        add_at(&mut buf, start, &bass_note(*f, q * 1.2, 0.35), 1.0);
        if b % 4 == 0 || b % 4 == 2 {
            add_at(&mut buf, start, &kick_s, 1.0);
        }
        if b % 4 == 1 || b % 4 == 3 {
            add_at(&mut buf, start, &snare_s, 1.0);
        }
        add_at(&mut buf, start, &hat_s, 1.0);
        add_at(&mut buf, start + (q * 0.5 * sr()) as usize, &hat_s, 0.6);
    }

    reverb(&mut buf, 0.16);
    fold_loop(&mut buf, len);
    normalize(&mut buf, 0.8);
    buf
}

fn crunches(buf: &mut Vec<f32>) {
    // one little bite per column as the cats pass (matches the animation)
    for c in 0..10u32 {
        let t = (c as f32 + 0.6) / 11.0 * CLEAR_DUR;
        add_at(buf, (t * sr()) as usize, &noise_burst(0.03, 0.30, 110.0, true, 100 + c), 1.0);
    }
}

fn build_all() -> SoundData {
    let mv = blip(900.0, 650.0, 0.05, 0.30);
    let rot = blip(520.0, 980.0, 0.07, 0.30);

    let mut lock = blip(170.0, 60.0, 0.12, 0.60);
    add_at(&mut lock, 0, &noise_burst(0.02, 0.15, 150.0, true, 3), 1.0);

    let mut drop = blip(320.0, 40.0, 0.2, 0.70);
    add_at(&mut drop, 0, &noise_burst(0.08, 0.30, 40.0, false, 4), 1.0);

    // 1-3 lines: two meows while the cats munch
    let mut clear = vec![0.0f32; (0.95 * sr()) as usize];
    add_at(&mut clear, 0, &meow_buf(600.0, 0.45, 0.8, Voice::Miau, 11), 1.0);
    add_at(&mut clear, (0.45 * sr()) as usize, &meow_buf(700.0, 0.42, 0.8, Voice::Miau, 23), 1.0);
    crunches(&mut clear);
    reverb(&mut clear, 0.2);
    normalize(&mut clear, 0.85);

    // 4 lines: three rising meows
    let mut tetris = vec![0.0f32; (1.0 * sr()) as usize];
    add_at(&mut tetris, 0, &meow_buf(560.0, 0.30, 0.8, Voice::Miau, 31), 1.0);
    add_at(&mut tetris, (0.30 * sr()) as usize, &meow_buf(680.0, 0.30, 0.8, Voice::Miau, 32), 1.0);
    add_at(&mut tetris, (0.62 * sr()) as usize, &meow_buf(820.0, 0.40, 0.85, Voice::Mew, 33), 1.0);
    crunches(&mut tetris);
    reverb(&mut tetris, 0.22);
    normalize(&mut tetris, 0.9);

    let mut level = Vec::new();
    for (i, f) in [523.25f32, 659.25, 783.99, 1046.5, 1318.5].iter().enumerate() {
        add_at(&mut level, (i as f32 * 0.09 * sr()) as usize, &pluck(*f, 0.5, 0.3), 1.0);
    }
    reverb(&mut level, 0.25);
    normalize(&mut level, 0.7);

    let mut over = meow_buf(430.0, 1.5, 0.9, Voice::Sad, 5);
    reverb(&mut over, 0.3);
    normalize(&mut over, 0.8);

    let mew = meow_buf(1050.0, 0.30, 0.6, Voice::Mew, 9);

    let mut boom = noise_burst(0.7, 0.9, 6.0, false, 21);
    add_at(&mut boom, 0, &blip(120.0, 30.0, 0.5, 0.9), 1.0);
    add_at(&mut boom, 0, &noise_burst(0.3, 0.35, 15.0, true, 22), 1.0);
    reverb(&mut boom, 0.25);
    normalize(&mut boom, 0.9);

    SoundData { mv, rot, lock, drop, clear, tetris, level, over, mew, boom, music: build_music() }
}

struct Sfx(Option<Sound>);

impl Sfx {
    async fn new(samples: Vec<f32>) -> Sfx {
        // If there is no audio device this stays None and the game is silent.
        Sfx(audio::load_sound_from_bytes(&to_wav(&samples)).await.ok())
    }
    fn play(&self) {
        if let Some(s) = &self.0 {
            play_sound_once(s);
        }
    }
    fn start_loop(&self, volume: f32) {
        if let Some(s) = &self.0 {
            play_sound(s, PlaySoundParams { looped: true, volume });
        }
    }
    fn stop(&self) {
        if let Some(s) = &self.0 {
            stop_sound(s);
        }
    }
}

struct Sounds {
    mv: Sfx,
    rot: Sfx,
    lock: Sfx,
    drop: Sfx,
    clear: Sfx,
    tetris: Sfx,
    level: Sfx,
    over: Sfx,
    mew: Sfx,
    boom: Sfx,
    music: Sfx,
}

impl Sounds {
    async fn load() -> Self {
        let d = build_all();
        Sounds {
            mv: Sfx::new(d.mv).await,
            rot: Sfx::new(d.rot).await,
            lock: Sfx::new(d.lock).await,
            drop: Sfx::new(d.drop).await,
            clear: Sfx::new(d.clear).await,
            tetris: Sfx::new(d.tetris).await,
            level: Sfx::new(d.level).await,
            over: Sfx::new(d.over).await,
            mew: Sfx::new(d.mew).await,
            boom: Sfx::new(d.boom).await,
            music: Sfx::new(d.music).await,
        }
    }
}

// ------------------------------------------------------------------ game ----

#[derive(Clone, Copy, PartialEq)]
enum State {
    Title,
    Playing,
    Paused,
    Over,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max: f32,
    size: f32,
    grow: f32,
    grav: f32,
    glow: bool,
    color: Color,
}

struct Boom {
    x: f32,
    y: f32,
    t: f32,
}

struct FloatText {
    text: String,
    x: f32,
    y: f32,
    t: f32,
    size: f32,
    color: Color,
}

struct CellFlash {
    x: i32,
    y: i32,
    t: f32,
}

struct Game {
    board: [[u8; W]; H],
    cur: Piece,
    next: Piece,
    bag: Vec<usize>,
    score: u32,
    lines: u32,
    level: u32,
    start_level: u32,
    fall: f32,
    clearing: Vec<usize>,
    clear_timer: f32,
    eat_col: i32,
    particles: Vec<Particle>,
    booms: Vec<Boom>,
    floats: Vec<FloatText>,
    flashes: Vec<CellFlash>,
    screen_flash: f32,
    shake: f32,
    over: bool,
    tl: f32,
    tr: f32,
    td: f32,
}

fn gravity(level: u32) -> f32 {
    // NES frames-per-row table at 60 fps
    let frames = match level {
        0 => 48,
        1 => 43,
        2 => 38,
        3 => 33,
        4 => 28,
        5 => 23,
        6 => 18,
        7 => 13,
        8 => 8,
        9 => 6,
        10..=12 => 5,
        13..=15 => 4,
        16..=18 => 3,
        19..=28 => 2,
        _ => 1,
    };
    frames as f32 / 60.0
}

fn auto_repeat(key: KeyCode, t: &mut f32, dt: f32, delay: f32, rate: f32) -> bool {
    if is_key_pressed(key) {
        *t = -delay;
        return true;
    }
    if is_key_down(key) {
        *t += dt;
        if *t >= rate {
            *t -= rate;
            return true;
        }
    }
    false
}

impl Game {
    fn new(start_level: u32) -> Self {
        let mut bag = Vec::new();
        let a = Self::draw(&mut bag);
        let b = Self::draw(&mut bag);
        Game {
            board: [[0; W]; H],
            cur: Piece::new(a),
            next: Piece::new(b),
            bag,
            score: 0,
            lines: 0,
            level: start_level,
            start_level,
            fall: 0.0,
            clearing: Vec::new(),
            clear_timer: 0.0,
            eat_col: 0,
            particles: Vec::new(),
            booms: Vec::new(),
            floats: Vec::new(),
            flashes: Vec::new(),
            screen_flash: 0.0,
            shake: 0.0,
            over: false,
            tl: 0.0,
            tr: 0.0,
            td: 0.0,
        }
    }

    // 7-bag randomizer, plus an occasional cat piece
    fn draw(bag: &mut Vec<usize>) -> usize {
        if rand::gen_range(0u32, CAT_ONE_IN) == 0 {
            return CAT;
        }
        if bag.is_empty() {
            *bag = (0..7).collect();
            for i in (1..7).rev() {
                let j = rand::gen_range(0usize, i + 1);
                bag.swap(i, j);
            }
        }
        bag.pop().unwrap()
    }

    fn valid(&self, p: &Piece) -> bool {
        for (x, y) in p.cells() {
            if x < 0 || x >= W as i32 || y >= H as i32 {
                return false;
            }
            if y >= 0 && self.board[y as usize][x as usize] != 0 {
                return false;
            }
        }
        true
    }

    fn try_move(&mut self, dx: i32, dy: i32) -> bool {
        let mut p = self.cur;
        p.x += dx;
        p.y += dy;
        if self.valid(&p) {
            self.cur = p;
            true
        } else {
            false
        }
    }

    fn try_rotate(&mut self, cw: bool) -> bool {
        if self.cur.kind == 1 || self.cur.kind == CAT {
            return false; // O piece and the cat don't rotate
        }
        let r = self.cur.rotated(cw);
        for k in [0, -1, 1, -2, 2] {
            let mut p = r;
            p.x += k;
            if self.valid(&p) {
                self.cur = p;
                return true;
            }
        }
        let mut p = r;
        p.y -= 1; // floor kick
        if self.valid(&p) {
            self.cur = p;
            return true;
        }
        false
    }

    fn ghost(&self) -> Piece {
        let mut p = self.cur;
        loop {
            let mut q = p;
            q.y += 1;
            if self.valid(&q) {
                p = q;
            } else {
                return p;
            }
        }
    }

    fn spawn(&mut self, snd: &Sounds) {
        self.cur = self.next;
        let k = Self::draw(&mut self.bag);
        self.next = Piece::new(k);
        self.fall = 0.0;
        if self.cur.kind == CAT {
            snd.mew.play();
        }
        if !self.valid(&self.cur) {
            self.over = true;
        }
    }

    // ---- effects ----

    #[allow(clippy::too_many_arguments)]
    fn emit(&mut self, x: f32, y: f32, vx: f32, vy: f32, life: f32, size: f32, grow: f32, grav: f32, glow: bool, color: Color) {
        self.particles.push(Particle { x, y, vx, vy, life, max: life, size, grow, grav, glow, color });
    }

    fn cell_center(cx: i32, cy: i32) -> (f32, f32) {
        (BX + (cx as f32 + 0.5) * CELL, BY + (cy as f32 + 0.5) * CELL)
    }

    // debris squares
    fn burst(&mut self, cx: i32, cy: i32, color: Color, count: usize) {
        let (px, py) = Self::cell_center(cx, cy);
        for _ in 0..count {
            let life = rand::gen_range(0.4f32, 0.9);
            self.emit(
                px + rand::gen_range(-10.0f32, 10.0),
                py + rand::gen_range(-10.0f32, 10.0),
                rand::gen_range(-200.0f32, 200.0),
                rand::gen_range(-320.0f32, -20.0),
                life,
                rand::gen_range(3.0f32, 7.0),
                0.0,
                1.0,
                false,
                color,
            );
        }
    }

    // glowing sparks
    fn sparks(&mut self, px: f32, py: f32, count: usize) {
        for _ in 0..count {
            let ang = rand::gen_range(0.0f32, 2.0 * PI);
            let sp = rand::gen_range(120.0f32, 430.0);
            let life = rand::gen_range(0.3f32, 0.8);
            let color = if rand::gen_range(0u32, 2) == 0 {
                Color::new(1.0, 0.55, 0.1, 1.0)
            } else {
                Color::new(1.0, 0.9, 0.4, 1.0)
            };
            self.emit(px, py, ang.cos() * sp, ang.sin() * sp, life, rand::gen_range(2.0f32, 4.0), 0.0, 0.35, true, color);
        }
    }

    fn smoke(&mut self, px: f32, py: f32, count: usize) {
        for _ in 0..count {
            let life = rand::gen_range(0.7f32, 1.3);
            self.emit(
                px + rand::gen_range(-14.0f32, 14.0),
                py + rand::gen_range(-14.0f32, 14.0),
                rand::gen_range(-25.0f32, 25.0),
                rand::gen_range(-70.0f32, -25.0),
                life,
                rand::gen_range(8.0f32, 14.0),
                14.0,
                0.0,
                false,
                Color::new(0.36, 0.33, 0.40, 1.0),
            );
        }
    }

    fn float_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        self.floats.push(FloatText { text: text.to_string(), x, y, t: 0.0, size, color });
    }

    fn update_fx(&mut self, dt: f32) {
        for p in self.particles.iter_mut() {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.vy += 700.0 * p.grav * dt;
            p.size += p.grow * dt;
            p.life -= dt;
        }
        self.particles.retain(|p| p.life > 0.0);
        for b in self.booms.iter_mut() {
            b.t += dt;
        }
        self.booms.retain(|b| b.t < BOOM_DUR);
        for f in self.floats.iter_mut() {
            f.t += dt;
            f.y -= 38.0 * dt;
        }
        self.floats.retain(|f| f.t < FLOAT_DUR);
        for f in self.flashes.iter_mut() {
            f.t += dt;
        }
        self.flashes.retain(|f| f.t < LOCK_FLASH);
        self.screen_flash = (self.screen_flash - dt * 1.6).max(0.0);
        self.shake = (self.shake - dt).max(0.0);
    }

    // The cat landed: blow up a square area around it.
    fn explode(&mut self, snd: &Sounds) {
        let cx = self.cur.x;
        let cy = self.cur.y;
        let mut destroyed = 0u32;
        for dy in -BOMB_R..=BOMB_R {
            for dx in -BOMB_R..=BOMB_R {
                let x = cx + dx;
                let y = cy + dy;
                if x < 0 || x >= W as i32 || y < 0 || y >= H as i32 {
                    continue;
                }
                let v = self.board[y as usize][x as usize];
                if v != 0 {
                    self.board[y as usize][x as usize] = 0;
                    destroyed += 1;
                    self.burst(x, y, piece_color(v as usize - 1), 6);
                }
            }
        }
        let (px, py) = Self::cell_center(cx, cy);
        self.burst(cx, cy, piece_color(CAT), 12);
        self.sparks(px, py, 46);
        self.smoke(px, py, 12);
        self.booms.push(Boom { x: px, y: py, t: 0.0 });
        self.screen_flash = 0.35;
        self.shake = 0.35;
        let pts = destroyed * 10 * (self.level + 1);
        self.score += pts;
        self.float_text("BOOM!", px.clamp(BX + 50.0, BX + 250.0), py - 24.0, 34.0, Color::new(1.0, 0.7, 0.2, 1.0));
        if pts > 0 {
            self.float_text(&format!("+{}", pts), px.clamp(BX + 40.0, BX + 260.0), py + 8.0, 24.0, WHITE);
        }
        snd.boom.play();
        snd.mew.play();
        self.spawn(snd);
    }

    fn lock(&mut self, snd: &Sounds) {
        if self.cur.kind == CAT {
            self.explode(snd);
            return;
        }
        for (x, y) in self.cur.cells() {
            if y >= 0 {
                self.board[y as usize][x as usize] = self.cur.kind as u8 + 1;
                self.flashes.push(CellFlash { x, y, t: 0.0 });
            }
        }
        let rows: Vec<usize> = (0..H)
            .filter(|&r| self.board[r].iter().all(|&c| c != 0))
            .collect();
        if rows.is_empty() {
            snd.lock.play();
            self.spawn(snd);
        } else {
            if rows.len() >= 4 {
                snd.tetris.play();
            } else {
                snd.clear.play();
            }
            self.clearing = rows;
            self.clear_timer = 0.0;
            self.eat_col = 0;
        }
    }

    // Cats run along the cleared rows from left to right, eating each block.
    fn update_clear(&mut self, dt: f32, snd: &Sounds) {
        self.clear_timer += dt;
        let p = (self.clear_timer / CLEAR_DUR).min(1.0);
        let cat_pos = p * (W as f32 + 1.0);
        let eaten = ((cat_pos + 0.4) as i32).min(W as i32);
        if eaten > self.eat_col {
            let rows = self.clearing.clone();
            for c in self.eat_col..eaten {
                for &r in &rows {
                    let v = self.board[r][c as usize];
                    if v != 0 {
                        self.burst(c, r as i32, piece_color(v as usize - 1), 5);
                    }
                }
            }
            self.eat_col = eaten;
        }
        if self.clear_timer >= CLEAR_DUR {
            self.finish_clear(snd);
        }
    }

    fn finish_clear(&mut self, snd: &Sounds) {
        let rows = std::mem::take(&mut self.clearing);
        let n = rows.len();
        let old_level = self.level;
        let pts = [0u32, 40, 100, 300, 1200][n.min(4)] * (self.level + 1);
        self.score += pts;
        let mid = rows.iter().sum::<usize>() as f32 / n.max(1) as f32;
        let fy = BY + mid * CELL;
        let fx = BX + W as f32 * CELL / 2.0;
        let words = ["", "MEOW!", "MIAU MIAU!", "PURRRR!", "PURRFECT!"];
        self.float_text(words[n.min(4)], fx, fy - 6.0, 36.0, Color::new(1.0, 0.85, 0.3, 1.0));
        self.float_text(&format!("+{}", pts), fx, fy + 24.0, 26.0, WHITE);
        for &r in &rows {
            for i in (1..=r).rev() {
                self.board[i] = self.board[i - 1];
            }
            self.board[0] = [0; W];
        }
        self.lines += n as u32;
        self.level = self.start_level + self.lines / 10;
        if self.level > old_level {
            snd.level.play();
            let lvl = format!("LEVEL {}", self.level);
            self.float_text(&lvl, fx, BY + 140.0, 40.0, Color::new(0.4, 0.95, 1.0, 1.0));
        }
        self.spawn(snd);
    }
}

// --------------------------------------------------------------- drawing ----

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
    draw_rectangle(x + r, y, w - 2.0 * r, h, c);
    draw_rectangle(x, y + r, r, h - 2.0 * r, c);
    draw_rectangle(x + w - r, y + r, r, h - 2.0 * r, c);
    draw_circle(x + r, y + r, r, c);
    draw_circle(x + w - r, y + r, r, c);
    draw_circle(x + r, y + h - r, r, c);
    draw_circle(x + w - r, y + h - r, r, c);
}

fn panel(x: f32, y: f32, w: f32, h: f32) {
    rrect(x, y, w, h, 9.0, Color::new(0.30, 0.28, 0.52, 1.0));
    rrect(x + 2.0, y + 2.0, w - 4.0, h - 4.0, 7.5, Color::new(0.07, 0.07, 0.15, 1.0));
}

fn draw_paw(cx: f32, cy: f32, s: f32, c: Color) {
    draw_circle(cx, cy + 0.18 * s, 0.30 * s, c);
    for (dx, dy, r) in [(-0.34f32, -0.02f32, 0.13f32), (-0.12, -0.24, 0.14), (0.12, -0.24, 0.14), (0.34, -0.02, 0.13)] {
        draw_circle(cx + dx * s, cy + dy * s, r * s, c);
    }
}

fn bg_color(f: f32) -> Color {
    Color::new(0.05 + 0.10 * f, 0.04 + 0.05 * f, 0.15 + 0.13 * f, 1.0)
}

fn draw_background(t: f32) {
    let steps = 70;
    draw_rectangle(-30.0, -30.0, WIN_W + 60.0, 31.0, bg_color(0.0));
    for i in 0..steps {
        let f = i as f32 / steps as f32;
        draw_rectangle(-30.0, f * WIN_H, WIN_W + 60.0, WIN_H / steps as f32 + 1.0, bg_color(f));
    }
    draw_rectangle(-30.0, WIN_H, WIN_W + 60.0, 30.0, bg_color(1.0));

    for i in 0..70 {
        let a = ((i as f32 + 1.0) * 12.9898).sin() * 43758.547;
        let b = ((i as f32 + 1.0) * 78.233).sin() * 12345.678;
        let x = (a - a.floor()) * WIN_W;
        let y = (b - b.floor()) * WIN_H * 0.9;
        let tw = 0.5 + 0.5 * (t * (0.8 + (i % 5) as f32 * 0.35) + i as f32).sin();
        let r = 0.7 + (i % 3) as f32 * 0.45;
        draw_circle(x, y, r, Color::new(1.0, 1.0, 0.92, 0.18 + 0.55 * tw));
    }

    // crescent moon
    draw_circle(556.0, 38.0, 24.0, Color::new(1.0, 0.95, 0.7, 0.07));
    draw_circle(556.0, 38.0, 15.0, Color::new(0.98, 0.94, 0.78, 1.0));
    draw_circle(563.0, 33.0, 13.0, bg_color(38.0 / WIN_H));
}

// Glossy block with bevel, shine and a faint paw print.
fn draw_block(px: f32, py: f32, size: f32, c: Color) {
    let dark = Color::new(c.r * 0.30, c.g * 0.30, c.b * 0.30, 1.0);
    let mid = Color::new(c.r * 0.72, c.g * 0.72, c.b * 0.72, 1.0);
    let light = Color::new((c.r + 0.5).min(1.0), (c.g + 0.5).min(1.0), (c.b + 0.5).min(1.0), 1.0);
    draw_rectangle(px, py, size, size, dark);
    draw_rectangle(px + 1.0, py + 1.0, size - 2.0, size - 2.0, mid);
    draw_rectangle(px + 2.5, py + 2.5, size - 5.0, size - 5.0, c);
    draw_rectangle(px + 1.0, py + 1.0, size - 2.0, 2.0, Color::new(light.r, light.g, light.b, 0.85));
    draw_rectangle(px + 1.0, py + 1.0, 2.0, size - 2.0, Color::new(light.r, light.g, light.b, 0.6));
    draw_rectangle(px + 3.0, py + 3.0, size - 6.0, (size - 6.0) * 0.38, Color::new(1.0, 1.0, 1.0, 0.14));
    draw_circle(px + size * 0.27, py + size * 0.27, size * 0.07, Color::new(1.0, 1.0, 1.0, 0.55));
    draw_paw(px + size * 0.5, py + size * 0.54, size * 0.46, Color::new(1.0, 1.0, 1.0, 0.13));
}

// Front-facing cat head with a lit fuse. It blinks red: it is about to explode.
fn draw_cat_face(px: f32, py: f32, s: f32) {
    let t = get_time() as f32;
    let flash = ((t * 7.0) as i32) % 2 == 0;
    let fur = if flash { Color::new(1.0, 0.50, 0.40, 1.0) } else { Color::new(0.74, 0.74, 0.80, 1.0) };
    let dark = Color::new(fur.r * 0.55, fur.g * 0.55, fur.b * 0.55, 1.0);
    let light = Color::new((fur.r + 0.18).min(1.0), (fur.g + 0.18).min(1.0), (fur.b + 0.18).min(1.0), 1.0);
    let pink = Color::new(1.0, 0.62, 0.72, 1.0);
    let cx = px + s * 0.5;
    let cy = py + s * 0.58;

    // ears
    draw_triangle(vec2(px + 0.04 * s, py + 0.52 * s), vec2(px + 0.08 * s, py - 0.02 * s), vec2(px + 0.50 * s, py + 0.26 * s), dark);
    draw_triangle(vec2(px + 0.96 * s, py + 0.52 * s), vec2(px + 0.92 * s, py - 0.02 * s), vec2(px + 0.50 * s, py + 0.26 * s), dark);
    draw_triangle(vec2(px + 0.09 * s, py + 0.50 * s), vec2(px + 0.12 * s, py + 0.05 * s), vec2(px + 0.46 * s, py + 0.27 * s), fur);
    draw_triangle(vec2(px + 0.91 * s, py + 0.50 * s), vec2(px + 0.88 * s, py + 0.05 * s), vec2(px + 0.54 * s, py + 0.27 * s), fur);
    draw_triangle(vec2(px + 0.16 * s, py + 0.42 * s), vec2(px + 0.17 * s, py + 0.15 * s), vec2(px + 0.37 * s, py + 0.29 * s), pink);
    draw_triangle(vec2(px + 0.84 * s, py + 0.42 * s), vec2(px + 0.83 * s, py + 0.15 * s), vec2(px + 0.63 * s, py + 0.29 * s), pink);

    // head
    draw_circle(cx, cy, 0.46 * s, dark);
    draw_circle(cx, cy, 0.43 * s, fur);
    draw_circle(cx - 0.12 * s, cy + 0.15 * s, 0.14 * s, light);
    draw_circle(cx + 0.12 * s, cy + 0.15 * s, 0.14 * s, light);
    // tabby stripes
    for dx in [-0.11f32, 0.0, 0.11] {
        draw_line(cx + dx * s, cy - 0.42 * s, cx + dx * 1.3 * s, cy - 0.27 * s, (0.04 * s).max(1.2), dark);
    }
    // eyes: green iris, slit pupil, highlight
    let iris = if flash { Color::new(1.0, 0.85, 0.2, 1.0) } else { Color::new(0.45, 0.95, 0.35, 1.0) };
    for ex in [cx - 0.17 * s, cx + 0.17 * s] {
        let ey = cy - 0.04 * s;
        draw_circle(ex, ey, 0.105 * s, Color::new(0.1, 0.1, 0.12, 1.0));
        draw_circle(ex, ey, 0.09 * s, iris);
        draw_ellipse(ex, ey, 0.028 * s, 0.08 * s, 0.0, BLACK);
        draw_circle(ex - 0.03 * s, ey - 0.04 * s, 0.025 * s, WHITE);
    }
    // nose, mouth, whiskers
    draw_triangle(vec2(cx - 0.05 * s, cy + 0.06 * s), vec2(cx + 0.05 * s, cy + 0.06 * s), vec2(cx, cy + 0.12 * s), pink);
    let mc = Color::new(0.2, 0.15, 0.2, 1.0);
    draw_line(cx, cy + 0.12 * s, cx, cy + 0.17 * s, 1.0, mc);
    draw_line(cx, cy + 0.17 * s, cx - 0.06 * s, cy + 0.2 * s, 1.0, mc);
    draw_line(cx, cy + 0.17 * s, cx + 0.06 * s, cy + 0.2 * s, 1.0, mc);
    let wc = Color::new(0.95, 0.95, 1.0, 0.8);
    for sgn in [-1.0f32, 1.0] {
        draw_line(cx + sgn * 0.14 * s, cy + 0.10 * s, cx + sgn * 0.46 * s, cy + 0.03 * s, 1.0, wc);
        draw_line(cx + sgn * 0.14 * s, cy + 0.14 * s, cx + sgn * 0.46 * s, cy + 0.17 * s, 1.0, wc);
    }

    // lit fuse between the ears
    let fuse = Color::new(0.45, 0.30, 0.15, 1.0);
    let x0 = cx;
    let y0 = py + 0.17 * s;
    let x1 = cx + 0.08 * s;
    let y1 = py + 0.02 * s;
    let x2 = cx + 0.02 * s;
    let y2 = py - 0.12 * s;
    draw_line(x0, y0, x1, y1, 2.0, fuse);
    draw_line(x1, y1, x2, y2, 2.0, fuse);
    let fl = 0.8 + 0.4 * (t * 30.0).sin().abs();
    draw_circle(x2, y2, 0.13 * s * fl, Color::new(1.0, 0.55, 0.1, 0.35));
    draw_circle(x2, y2, 0.065 * s * fl, Color::new(1.0, 0.9, 0.3, 1.0));
    draw_circle(x2, y2, 0.03 * s, WHITE);
}

// One unit of any piece kind (colored block or the cat).
fn draw_unit(px: f32, py: f32, size: f32, kind: usize) {
    if kind == CAT {
        draw_cat_face(px, py, size);
    } else {
        draw_block(px, py, size, piece_color(kind));
    }
}

// Side-view cat running along a row, eating the blocks.
fn draw_cat_runner(x: f32, y: f32, s: f32, t: f32, variant: usize) {
    let (fur, dark, light) = match variant % 4 {
        0 => (Color::new(0.96, 0.60, 0.22, 1.0), Color::new(0.72, 0.36, 0.10, 1.0), Color::new(1.0, 0.86, 0.62, 1.0)),
        1 => (Color::new(0.62, 0.62, 0.68, 1.0), Color::new(0.34, 0.34, 0.42, 1.0), Color::new(0.88, 0.88, 0.92, 1.0)),
        2 => (Color::new(0.96, 0.96, 0.98, 1.0), Color::new(0.68, 0.68, 0.78, 1.0), WHITE),
        _ => (Color::new(0.20, 0.20, 0.25, 1.0), Color::new(0.08, 0.08, 0.10, 1.0), Color::new(0.42, 0.42, 0.50, 1.0)),
    };
    let pink = Color::new(1.0, 0.62, 0.72, 1.0);
    let ph = t * 16.0;
    let bob = -ph.sin().abs() * 0.04 * s;

    // tail
    let mut qx = x + 0.14 * s;
    let mut qy = y + 0.46 * s + bob;
    for k in 1..=4 {
        let kk = k as f32;
        let nx = x + 0.14 * s - kk * 0.06 * s;
        let ny = y + 0.46 * s + bob - kk * 0.09 * s + (ph * 0.7 + kk).sin() * 0.04 * s;
        draw_line(qx, qy, nx, ny, 0.09 * s, fur);
        qx = nx;
        qy = ny;
    }

    // legs: far pair, body, near pair
    let leg = |ox: f32, off: f32, col: Color| {
        let sw = (ph + off).sin() * 0.07 * s;
        let lift = (ph + off).cos().max(0.0) * 0.05 * s;
        draw_line(x + ox * s, y + 0.68 * s + bob, x + ox * s + sw, y + 0.95 * s - lift, 0.10 * s, col);
        draw_circle(x + ox * s + sw, y + 0.96 * s - lift, 0.055 * s, col);
    };
    leg(0.16, 0.0, dark);
    leg(0.50, PI, dark);

    draw_rectangle(x + 0.12 * s, y + 0.30 * s + bob, 0.52 * s, 0.42 * s, fur);
    draw_circle(x + 0.12 * s, y + 0.51 * s + bob, 0.21 * s, fur);
    draw_rectangle(x + 0.16 * s, y + 0.64 * s + bob, 0.48 * s, 0.08 * s, light);
    for i in 0..3 {
        let sx = x + (0.24 + 0.12 * i as f32) * s;
        draw_line(sx, y + 0.30 * s + bob, sx + 0.02 * s, y + 0.46 * s + bob, 0.045 * s, dark);
    }

    leg(0.26, PI, fur);
    leg(0.58, 0.0, fur);

    // head
    let hx = x + 0.76 * s;
    let hy = y + 0.40 * s + bob;
    draw_circle(hx, hy, 0.26 * s, fur);
    draw_triangle(vec2(hx - 0.20 * s, hy - 0.10 * s), vec2(hx - 0.14 * s, hy - 0.40 * s), vec2(hx, hy - 0.20 * s), fur);
    draw_triangle(vec2(hx + 0.02 * s, hy - 0.20 * s), vec2(hx + 0.18 * s, hy - 0.40 * s), vec2(hx + 0.24 * s, hy - 0.08 * s), fur);
    draw_triangle(vec2(hx - 0.15 * s, hy - 0.14 * s), vec2(hx - 0.13 * s, hy - 0.33 * s), vec2(hx - 0.04 * s, hy - 0.20 * s), pink);
    draw_triangle(vec2(hx + 0.06 * s, hy - 0.20 * s), vec2(hx + 0.17 * s, hy - 0.33 * s), vec2(hx + 0.20 * s, hy - 0.11 * s), pink);
    // eye
    draw_circle(hx + 0.09 * s, hy - 0.05 * s, 0.065 * s, Color::new(0.5, 0.95, 0.35, 1.0));
    draw_ellipse(hx + 0.10 * s, hy - 0.05 * s, 0.02 * s, 0.055 * s, 0.0, BLACK);
    draw_circle(hx + 0.075 * s, hy - 0.075 * s, 0.018 * s, WHITE);
    // chewing mouth
    if ((t * 10.0) as i32) % 2 == 0 {
        draw_triangle(
            vec2(hx + 0.26 * s, hy),
            vec2(hx + 0.02 * s, hy + 0.07 * s),
            vec2(hx + 0.26 * s, hy + 0.22 * s),
            Color::new(0.55, 0.05, 0.08, 1.0),
        );
    }
    draw_circle(hx + 0.24 * s, hy - 0.02 * s, 0.03 * s, pink);
    let wc = Color::new(1.0, 1.0, 1.0, 0.75);
    draw_line(hx + 0.18 * s, hy + 0.04 * s, hx + 0.40 * s, hy - 0.02 * s, 1.0, wc);
    draw_line(hx + 0.18 * s, hy + 0.08 * s, hx + 0.40 * s, hy + 0.12 * s, 1.0, wc);
}

fn center_text(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let d = measure_text(text, None, size as u16, 1.0);
    draw_text(text, cx - d.width / 2.0, y, size, color);
}

fn shadow_text(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    center_text(text, cx + 2.0, y + 2.0, size, Color::new(0.0, 0.0, 0.0, color.a * 0.8));
    center_text(text, cx, y, size, color);
}

fn draw_next(p: &Piece, cx: f32, cy: f32) {
    if p.kind == CAT {
        draw_cat_face(cx - 22.0, cy - 18.0, 44.0);
        return;
    }
    let s = 24.0;
    let (mut minr, mut maxr, mut minc, mut maxc) = (9usize, 0usize, 9usize, 0usize);
    for r in 0..p.n {
        for c in 0..p.n {
            if p.g[r][c] {
                minr = minr.min(r);
                maxr = maxr.max(r);
                minc = minc.min(c);
                maxc = maxc.max(c);
            }
        }
    }
    let w = (maxc - minc + 1) as f32 * s;
    let h = (maxr - minr + 1) as f32 * s;
    for r in 0..p.n {
        for c in 0..p.n {
            if p.g[r][c] {
                let x = cx - w / 2.0 + (c - minc) as f32 * s;
                let y = cy - h / 2.0 + (r - minr) as f32 * s;
                draw_block(x, y, s, piece_color(p.kind));
            }
        }
    }
}

fn draw_game(game: &Game, state: State, hi: u32, start_level: u32, music_on: bool) {
    let t = get_time() as f32;

    // screen shake via a shifted camera
    let (sx, sy) = if game.shake > 0.0 {
        let m = game.shake * 24.0;
        ((t * 90.0).sin() * m, (t * 77.0).cos() * m)
    } else {
        (0.0, 0.0)
    };
    set_camera(&Camera2D {
        target: vec2(sx + WIN_W / 2.0, sy + WIN_H / 2.0),
        zoom: vec2(2.0 / WIN_W, 2.0 / WIN_H),
        ..Default::default()
    });

    draw_background(t);
    shadow_text("CAT TETRIS", BX + W as f32 * CELL / 2.0, 36.0, 40.0, Color::new(1.0, 0.86, 0.55, 1.0));

    // well
    let bw = W as f32 * CELL;
    let bh = H as f32 * CELL;
    rrect(BX - 9.0, BY - 9.0, bw + 18.0, bh + 18.0, 9.0, Color::new(0.34, 0.31, 0.58, 1.0));
    rrect(BX - 6.0, BY - 6.0, bw + 12.0, bh + 12.0, 7.0, Color::new(0.10, 0.09, 0.20, 1.0));
    for i in 0..20 {
        let f = i as f32 / 20.0;
        draw_rectangle(BX, BY + f * bh, bw, bh / 20.0 + 1.0, Color::new(0.03 + 0.04 * f, 0.03 + 0.02 * f, 0.07 + 0.06 * f, 1.0));
    }
    // faint paw prints drifting up behind the blocks
    for i in 0..5 {
        let sp = 14.0 + 3.0 * i as f32;
        let y = BY + bh - ((t * sp + i as f32 * 140.0) % (bh + 60.0)) + 30.0;
        let x = BX + 30.0 + ((i * 61) % 240) as f32;
        if y > BY + 14.0 && y < BY + bh - 14.0 {
            draw_paw(x, y, 38.0, Color::new(1.0, 1.0, 1.0, 0.045));
        }
    }
    for i in 1..W {
        draw_line(BX + i as f32 * CELL, BY, BX + i as f32 * CELL, BY + bh, 1.0, Color::new(1.0, 1.0, 1.0, 0.045));
    }
    for i in 1..H {
        draw_line(BX, BY + i as f32 * CELL, BX + bw, BY + i as f32 * CELL, 1.0, Color::new(1.0, 1.0, 1.0, 0.045));
    }

    // settled blocks (blocks already passed by a cat are gone)
    for r in 0..H {
        let eating = game.clearing.contains(&r);
        for c in 0..W {
            let v = game.board[r][c];
            if v == 0 {
                continue;
            }
            if eating && (c as i32) < game.eat_col {
                continue;
            }
            draw_block(BX + c as f32 * CELL, BY + r as f32 * CELL, CELL, piece_color(v as usize - 1));
        }
    }
    for f in &game.flashes {
        let a = 0.55 * (1.0 - f.t / LOCK_FLASH);
        draw_rectangle(BX + f.x as f32 * CELL, BY + f.y as f32 * CELL, CELL, CELL, Color::new(1.0, 1.0, 1.0, a));
    }

    // ghost + current piece
    if state != State::Title && game.clearing.is_empty() {
        let g = game.ghost();
        let col = piece_color(game.cur.kind);
        for (x, y) in g.cells() {
            if y >= 0 {
                let gx = BX + x as f32 * CELL;
                let gy = BY + y as f32 * CELL;
                draw_rectangle(gx + 2.0, gy + 2.0, CELL - 4.0, CELL - 4.0, Color::new(col.r, col.g, col.b, 0.10));
                draw_rectangle_lines(gx + 2.0, gy + 2.0, CELL - 4.0, CELL - 4.0, 2.0, Color::new(col.r, col.g, col.b, 0.55));
            }
        }
        // show the blast area where the cat will land
        if game.cur.kind == CAT {
            let x0 = (g.x - BOMB_R).max(0);
            let x1 = (g.x + BOMB_R).min(W as i32 - 1);
            let y0 = (g.y - BOMB_R).max(0);
            let y1 = (g.y + BOMB_R).min(H as i32 - 1);
            let pulse = 0.10 + 0.06 * (t * 8.0).sin();
            draw_rectangle(
                BX + x0 as f32 * CELL,
                BY + y0 as f32 * CELL,
                (x1 - x0 + 1) as f32 * CELL,
                (y1 - y0 + 1) as f32 * CELL,
                Color::new(1.0, 0.2, 0.1, pulse),
            );
            draw_rectangle_lines(
                BX + x0 as f32 * CELL,
                BY + y0 as f32 * CELL,
                (x1 - x0 + 1) as f32 * CELL,
                (y1 - y0 + 1) as f32 * CELL,
                2.0,
                Color::new(1.0, 0.3, 0.2, 0.75),
            );
        }
        for (x, y) in game.cur.cells() {
            if y >= 0 {
                draw_unit(BX + x as f32 * CELL, BY + y as f32 * CELL, CELL, game.cur.kind);
            }
        }
    }

    // cats eating the cleared rows
    if !game.clearing.is_empty() {
        let p = (game.clear_timer / CLEAR_DUR).min(1.0);
        let cat_pos = p * (W as f32 + 1.0);
        let x = BX + (cat_pos - 1.0) * CELL;
        for (i, &r) in game.clearing.iter().enumerate() {
            draw_cat_runner(x, BY + r as f32 * CELL, CELL, t + i as f32 * 0.3, r + i);
        }
    }

    // explosions, smoke, sparks and crumbs
    for b in &game.booms {
        let p = b.t / BOOM_DUR;
        let a = 1.0 - p;
        let radius = (p * (BOMB_R as f32 + 1.0) * CELL).max(1.0);
        draw_circle(b.x, b.y, radius, Color::new(1.0, 0.45, 0.1, 0.38 * a));
        draw_circle(b.x, b.y, radius * 0.62, Color::new(1.0, 0.8, 0.3, 0.55 * a));
        draw_circle(b.x, b.y, radius * 0.3 * a + 2.0, Color::new(1.0, 1.0, 0.9, 0.9 * a * a));
        draw_circle_lines(b.x, b.y, radius, 4.0 * a + 1.0, Color::new(1.0, 0.95, 0.8, a));
    }
    for p in &game.particles {
        let a = (p.life / p.max).clamp(0.0, 1.0);
        let c = p.color;
        if p.glow {
            draw_circle(p.x, p.y, p.size * 2.6, Color::new(c.r, c.g, c.b, a * 0.22));
            draw_circle(p.x, p.y, p.size, Color::new(c.r, c.g, c.b, a));
        } else if p.grow > 0.0 {
            draw_circle(p.x, p.y, p.size, Color::new(c.r, c.g, c.b, a * 0.45));
        } else {
            draw_rectangle(p.x - p.size / 2.0, p.y - p.size / 2.0, p.size, p.size, Color::new(c.r, c.g, c.b, a));
        }
    }

    // floating text
    for f in &game.floats {
        let a = (1.0 - f.t / FLOAT_DUR).clamp(0.0, 1.0);
        let pop = 1.0 + 0.3 * (1.0 - (f.t * 6.0).min(1.0));
        shadow_text(&f.text, f.x, f.y, f.size * pop, Color::new(f.color.r, f.color.g, f.color.b, a));
    }

    // ---- side panel ----
    let label = Color::new(0.62, 0.66, 0.90, 1.0);
    let pw = 190.0;
    panel(PANEL_X, 60.0, pw, 125.0);
    center_text("NEXT", PANEL_X + pw / 2.0, 84.0, 22.0, label);
    if state != State::Title {
        draw_next(&game.next, PANEL_X + pw / 2.0, 138.0);
    }

    let shown_level = if state == State::Title { start_level } else { game.level };
    let boxes: [(&str, String, f32); 3] = [
        ("SCORE", format!("{}", game.score), 197.0),
        ("LINES", format!("{}", game.lines), 267.0),
        ("HIGH SCORE", format!("{}", hi), 440.0),
    ];
    for (name, val, y) in boxes.iter() {
        panel(PANEL_X, *y, pw, 62.0);
        draw_text(name, PANEL_X + 14.0, *y + 22.0, 19.0, label);
        draw_text(val, PANEL_X + 14.0, *y + 52.0, 32.0, WHITE);
    }
    // level box with progress bar toward the next level
    panel(PANEL_X, 337.0, pw, 90.0);
    draw_text("LEVEL", PANEL_X + 14.0, 359.0, 19.0, label);
    draw_text(&format!("{}", shown_level), PANEL_X + 14.0, 391.0, 32.0, WHITE);
    let frac = if state == State::Title { 0.0 } else { (game.lines % 10) as f32 / 10.0 };
    rrect(PANEL_X + 14.0, 402.0, pw - 28.0, 10.0, 5.0, Color::new(0.16, 0.16, 0.30, 1.0));
    if frac > 0.0 {
        rrect(PANEL_X + 14.0, 402.0, ((pw - 28.0) * frac).max(10.0), 10.0, 5.0, Color::new(0.35, 0.9, 1.0, 1.0));
    }

    let help = Color::new(0.65, 0.66, 0.80, 1.0);
    let lines = [
        "LEFT / RIGHT  move",
        "DOWN  soft drop",
        "UP or X  rotate",
        "Z  rotate back",
        "SPACE  hard drop",
        "P  pause",
        if music_on { "M  music: ON" } else { "M  music: OFF" },
    ];
    for (i, l) in lines.iter().enumerate() {
        draw_text(l, PANEL_X, 530.0 + i as f32 * 19.0, 18.0, help);
    }
    draw_text("Cat pieces explode!", PANEL_X, 678.0, 20.0, Color::new(1.0, 0.7, 0.4, 1.0));

    // overlays
    let cx = BX + bw / 2.0;
    let overlay = || draw_rectangle(BX, BY, bw, bh, Color::new(0.0, 0.0, 0.03, 0.76));
    match state {
        State::Title => {
            overlay();
            let bob = (t * 3.0).sin() * 6.0;
            shadow_text("CAT TETRIS", cx, 205.0, 52.0, Color::new(0.98, 0.65, 0.28, 1.0));
            draw_cat_face(cx - 42.0, 240.0 + bob, 84.0);
            shadow_text("Start level", cx, 380.0, 26.0, WHITE);
            shadow_text(&format!("< {} >", start_level), cx, 425.0, 48.0, Color::new(0.3, 0.85, 0.95, 1.0));
            center_text("UP / DOWN to change", cx, 452.0, 20.0, GRAY);
            let blink = 0.65 + 0.35 * (t * 4.0).sin();
            shadow_text("Press ENTER to play", cx, 512.0, 28.0, Color::new(1.0, 1.0, 1.0, blink));
            center_text("ESC to quit", cx, 560.0, 20.0, GRAY);
        }
        State::Paused => {
            overlay();
            shadow_text("PAUSED", cx, BY + bh / 2.0, 50.0, WHITE);
            center_text("Press P to resume", cx, BY + bh / 2.0 + 35.0, 24.0, GRAY);
        }
        State::Over => {
            overlay();
            shadow_text("GAME OVER", cx, BY + bh / 2.0 - 20.0, 50.0, Color::new(0.95, 0.3, 0.3, 1.0));
            shadow_text(&format!("Score {}", game.score), cx, BY + bh / 2.0 + 25.0, 30.0, WHITE);
            center_text("Press ENTER", cx, BY + bh / 2.0 + 65.0, 24.0, GRAY);
        }
        State::Playing => {}
    }

    // white flash when a cat blows up
    if game.screen_flash > 0.0 {
        draw_rectangle(-30.0, -30.0, WIN_W + 60.0, WIN_H + 60.0, Color::new(1.0, 0.95, 0.85, (game.screen_flash * 1.1).min(0.4)));
    }

    set_default_camera();
}

// ------------------------------------------------------------------ main ----

fn conf() -> Conf {
    Conf {
        window_title: "Cat Tetris".to_owned(),
        window_width: 600,
        window_height: 700,
        window_resizable: false,
        high_dpi: true, // sharp on Retina displays (macOS)
        ..Default::default()
    }
}

// Developer-only: renders screenshots and dumps sounds (cargo run --features shots)
#[cfg(feature = "shots")]
async fn run_shots(snd: &Sounds, dir: String) {
    let _ = std::fs::create_dir_all(&dir);
    let d = build_all();
    for (name, v) in [
        ("mv", &d.mv), ("rot", &d.rot), ("lock", &d.lock), ("drop", &d.drop), ("clear", &d.clear),
        ("tetris", &d.tetris), ("level", &d.level), ("over", &d.over), ("mew", &d.mew),
        ("boom", &d.boom), ("music", &d.music),
    ] {
        let _ = std::fs::write(format!("{}/{}.wav", dir, name), to_wav(v));
    }

    let mut game = Game::new(0);
    // title
    for i in 0..4 {
        draw_game(&game, State::Title, 1234, 0, true);
        if i == 3 {
            get_screen_data().export_png(&format!("{}/title.png", dir));
        }
        next_frame().await;
    }
    // a mid-game board
    for r in 11..H {
        for c in 0..W {
            if (r * 7 + c * 3) % 5 != 0 {
                game.board[r][c] = ((r + c * 2) % 7) as u8 + 1;
            }
        }
    }
    game.cur = Piece::new(2);
    game.cur.y = 3;
    game.next = Piece::new(CAT);
    game.score = 4200;
    game.lines = 14;
    game.level = 1;
    for i in 0..4 {
        draw_game(&game, State::Playing, 9000, 0, true);
        if i == 3 {
            get_screen_data().export_png(&format!("{}/play.png", dir));
        }
        next_frame().await;
    }
    // falling cat with blast preview
    game.cur = Piece::new(CAT);
    game.cur.x = 4;
    game.cur.y = 6;
    for i in 0..4 {
        draw_game(&game, State::Playing, 9000, 0, true);
        if i == 3 {
            get_screen_data().export_png(&format!("{}/cat.png", dir));
        }
        next_frame().await;
    }
    // cats eating two rows
    game.cur = Piece::new(0);
    for r in 18..20 {
        for c in 0..W {
            game.board[r][c] = ((r + c) % 7) as u8 + 1;
        }
    }
    game.clearing = vec![18, 19];
    game.clear_timer = 0.0;
    game.eat_col = 0;
    let mut saved = false;
    while !game.clearing.is_empty() {
        let dt = get_frame_time().min(0.05).max(0.016);
        game.update_clear(dt, snd);
        game.update_fx(dt);
        draw_game(&game, State::Playing, 9000, 0, true);
        if !saved && game.clear_timer > 0.45 {
            get_screen_data().export_png(&format!("{}/eat.png", dir));
            saved = true;
        }
        next_frame().await;
    }
    // boom
    game.cur = Piece::new(CAT);
    game.cur.x = 4;
    game.cur.y = 13;
    game.lock(snd);
    let mut frames = 0;
    let mut saved1 = false;
    while frames < 60 {
        let dt = 0.016;
        game.update_fx(dt);
        draw_game(&game, State::Playing, 9000, 0, true);
        if !saved1 && game.booms.first().map(|b| b.t > 0.1).unwrap_or(false) {
            get_screen_data().export_png(&format!("{}/boom.png", dir));
            saved1 = true;
        }
        frames += 1;
        next_frame().await;
    }
    draw_game(&game, State::Playing, 9000, 0, true);
    get_screen_data().export_png(&format!("{}/after.png", dir));
    next_frame().await;
}

#[macroquad::main(conf)]
async fn main() {
    rand::srand((get_time() * 1_000_000.0) as u64 ^ 0x9E37_79B9);
    let snd = Sounds::load().await;

    #[cfg(feature = "shots")]
    if let Ok(dir) = std::env::var("CATTETRIS_SHOT") {
        run_shots(&snd, dir).await;
        return;
    }

    let mut state = State::Title;
    let mut game = Game::new(0);
    let mut start_level: u32 = 0;
    let mut music_on = true;
    let mut hi: u32 = 0;

    loop {
        let dt = get_frame_time().min(0.05);

        if is_key_pressed(KeyCode::Escape) {
            break;
        }
        if is_key_pressed(KeyCode::M) {
            music_on = !music_on;
            if music_on {
                if state == State::Playing {
                    snd.music.start_loop(0.45);
                }
            } else {
                snd.music.stop();
            }
        }

        match state {
            State::Title => {
                if is_key_pressed(KeyCode::Up) && start_level < 19 {
                    start_level += 1;
                    snd.mv.play();
                }
                if is_key_pressed(KeyCode::Down) && start_level > 0 {
                    start_level -= 1;
                    snd.mv.play();
                }
                if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                    game = Game::new(start_level);
                    state = State::Playing;
                    if music_on {
                        snd.music.start_loop(0.45);
                    }
                }
            }
            State::Paused => {
                if is_key_pressed(KeyCode::P) {
                    state = State::Playing;
                    if music_on {
                        snd.music.start_loop(0.45);
                    }
                }
            }
            State::Over => {
                if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                    game = Game::new(start_level);
                    state = State::Title;
                }
            }
            State::Playing => {
                if is_key_pressed(KeyCode::P) {
                    state = State::Paused;
                    snd.music.stop();
                } else if !game.clearing.is_empty() {
                    game.update_clear(dt, &snd);
                } else {
                    if auto_repeat(KeyCode::Left, &mut game.tl, dt, 0.17, 0.05) && game.try_move(-1, 0) {
                        snd.mv.play();
                    }
                    if auto_repeat(KeyCode::Right, &mut game.tr, dt, 0.17, 0.05) && game.try_move(1, 0) {
                        snd.mv.play();
                    }
                    if (is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::X)) && game.try_rotate(true) {
                        snd.rot.play();
                    }
                    if (is_key_pressed(KeyCode::Z) || is_key_pressed(KeyCode::LeftControl))
                        && game.try_rotate(false)
                    {
                        snd.rot.play();
                    }
                    if auto_repeat(KeyCode::Down, &mut game.td, dt, 0.0, 0.035) && game.try_move(0, 1) {
                        game.score += 1;
                        game.fall = 0.0;
                    }

                    // gravity
                    game.fall += dt;
                    let g = gravity(game.level);
                    if game.fall >= g {
                        game.fall -= g;
                        if !game.try_move(0, 1) {
                            game.lock(&snd);
                        }
                    }

                    // hard drop
                    if game.clearing.is_empty() && !game.over && is_key_pressed(KeyCode::Space) {
                        let mut d = 0;
                        while game.try_move(0, 1) {
                            d += 1;
                        }
                        game.score += 2 * d;
                        snd.drop.play();
                        game.lock(&snd);
                    }
                }

                if game.over {
                    state = State::Over;
                    hi = hi.max(game.score);
                    snd.music.stop();
                    snd.over.play();
                }
            }
        }

        game.update_fx(dt);
        hi = hi.max(game.score);
        draw_game(&game, state, hi, start_level, music_on);
        next_frame().await;
    }
}
