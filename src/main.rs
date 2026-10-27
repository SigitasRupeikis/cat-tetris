// Cat Tetris in Rust (macroquad).
// - Cleared rows are eaten by running cats.
// - Every so often a CAT piece drops. It is a single block that explodes on
//   landing and wipes out a square area around it.
// - All sounds and the cat-meow music are synthesized at startup (no assets).
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use macroquad::audio::{self, play_sound, play_sound_once, stop_sound, PlaySoundParams, Sound};
use macroquad::prelude::*;

const W: usize = 10;
const H: usize = 20;
const CELL: f32 = 30.0;
const BX: f32 = 40.0; // board origin
const BY: f32 = 50.0;
const PANEL_X: f32 = 375.0;

// ---- tuning ----
const CAT_ONE_IN: u32 = 10; // a cat piece appears on average once per 10 pieces
const BOMB_R: i32 = 2; // blast radius in cells: 2 = 5x5 area, 1 = 3x3 area
const CLEAR_DUR: f32 = 0.9; // seconds the cats take to eat the rows
const BOOM_DUR: f32 = 0.45;

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

// ----------------------------------------------------------------- sound ----

const SR: u32 = 22050;

fn tone(out: &mut Vec<f32>, freq: f32, dur: f32, vol: f32, duty: f32) {
    let n = (dur * SR as f32) as usize;
    let attack = 0.004 * SR as f32;
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let env = if freq <= 0.0 {
            0.0
        } else {
            (i as f32 / attack).min(1.0) * if t > 0.8 { (1.0 - t) / 0.2 } else { 1.0 }
        };
        let s = if phase < duty { 1.0 } else { -1.0 };
        out.push(s * env * vol);
        phase += freq / SR as f32;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
}

fn sweep(out: &mut Vec<f32>, f0: f32, f1: f32, dur: f32, vol: f32) {
    let n = (dur * SR as f32) as usize;
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let f = f0 + (f1 - f0) * t;
        let s = if phase < 0.5 { 1.0 } else { -1.0 };
        out.push(s * (1.0 - t) * vol);
        phase += f / SR as f32;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
}

fn noise(out: &mut Vec<f32>, dur: f32, vol: f32) {
    let n = (dur * SR as f32) as usize;
    let mut seed = 12345u32;
    let mut lp = 0.0f32;
    for i in 0..n {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let r = ((seed >> 16) as f32 / 32768.0) - 1.0;
        lp += 0.25 * (r - lp);
        let t = i as f32 / n as f32;
        out.push(lp * (1.0 - t).powi(2) * vol);
    }
}

fn mix(a: &mut Vec<f32>, b: &[f32]) {
    if b.len() > a.len() {
        a.resize(b.len(), 0.0);
    }
    for (i, s) in b.iter().enumerate() {
        a[i] += *s;
    }
}

// A synthesized "miau": quick pitch rise ("mi"), slow fall ("au"), with the
// pulse width sweeping to mimic the vowel change, vibrato and a soft low-pass.
fn meow(out: &mut Vec<f32>, base: f32, dur: f32, vol: f32) {
    let n = (dur * SR as f32) as usize;
    let mut phase = 0.0f32;
    let mut lp = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let contour = if t < 0.22 {
            0.7 + 0.3 * (t / 0.22)
        } else {
            1.0 - 0.45 * ((t - 0.22) / 0.78)
        };
        let vib = 1.0 + 0.025 * (i as f32 / SR as f32 * 45.0).sin();
        let f = base * contour * vib;
        let duty = 0.5 - 0.33 * (t * std::f32::consts::PI).sin();
        let raw = if phase < duty { 1.0 } else { -1.0 };
        lp += 0.3 * (raw - lp);
        let env = (t / 0.05).min(1.0) * ((1.0 - t) / 0.3).min(1.0);
        out.push(lp * env * vol);
        phase += f / SR as f32;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
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

enum Note {
    Tone(f32, f32), // frequency, beats
    Meow(f32, f32), // pitch, beats
    Rest(f32),
}

// Original bouncy tune with cats meowing "miau miau" in the melody.
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

    let melody = vec![
        // phrase 1
        Tone(E5, 0.5), Tone(G5, 0.5), Tone(E5, 0.5), Tone(C5, 0.5), Meow(750.0, 2.0),
        Tone(D5, 0.5), Tone(F5, 0.5), Tone(D5, 0.5), Tone(B4, 0.5), Meow(700.0, 2.0),
        // phrase 2
        Tone(C5, 0.5), Tone(E5, 0.5), Tone(G5, 0.5), Tone(E5, 0.5), Tone(A5, 1.0), Tone(G5, 1.0),
        Tone(E5, 0.5), Tone(D5, 0.5), Tone(C5, 0.5), Tone(D5, 0.5), Tone(C5, 1.5), Rest(0.5),
        // phrase 3
        Tone(A4, 0.5), Tone(C5, 0.5), Tone(E5, 0.5), Tone(C5, 0.5), Meow(650.0, 2.0),
        Tone(G4, 0.5), Tone(B4, 0.5), Tone(D5, 0.5), Tone(B4, 0.5), Meow(800.0, 2.0),
        // phrase 4
        Tone(E5, 0.5), Tone(G5, 0.5), Tone(A5, 0.5), Tone(G5, 0.5), Tone(E5, 1.0), Tone(C5, 1.0),
        Tone(D5, 0.5), Tone(E5, 0.5), Tone(D5, 0.5), Tone(B4, 0.5), Tone(C5, 1.5), Rest(0.5),
    ];

    let mut lead = Vec::new();
    for note in melody {
        match note {
            Tone(f, b) => tone(&mut lead, f, b * q, 0.15, 0.5),
            Meow(f, b) => meow(&mut lead, f, b * q, 0.30),
            Rest(b) => tone(&mut lead, 0.0, b * q, 0.0, 0.5),
        }
    }

    // one bass note per beat, 32 beats
    let bass_notes = [
        C3, G3, C3, G3, G2, D3, G2, D3, C3, G3, C3, G3, G2, D3, C3, C3,
        A2, E3, A2, E3, G2, D3, G2, D3, F3, C4, F3, C4, G2, D3, C3, C3,
    ];
    let mut bass = Vec::new();
    for f in bass_notes {
        tone(&mut bass, f, q, 0.09, 0.25);
    }

    mix(&mut lead, &bass);
    lead
}

struct Sounds {
    mv: Sound,
    rot: Sound,
    lock: Sound,
    drop: Sound,
    clear: Sound,
    tetris: Sound,
    level: Sound,
    over: Sound,
    mew: Sound,
    boom: Sound,
    music: Sound,
}

impl Sounds {
    async fn load() -> Self {
        async fn ld(v: Vec<f32>) -> Sound {
            audio::load_sound_from_bytes(&to_wav(&v))
                .await
                .expect("failed to load generated sound")
        }

        let mut v = Vec::new();
        tone(&mut v, 520.0, 0.03, 0.25, 0.5);
        let mv = ld(v).await;

        let mut v = Vec::new();
        tone(&mut v, 700.0, 0.035, 0.25, 0.25);
        tone(&mut v, 940.0, 0.035, 0.25, 0.25);
        let rot = ld(v).await;

        let mut v = Vec::new();
        sweep(&mut v, 200.0, 70.0, 0.09, 0.5);
        let lock = ld(v).await;

        let mut v = Vec::new();
        sweep(&mut v, 420.0, 50.0, 0.14, 0.55);
        let drop = ld(v).await;

        // "miau miau" while cats eat the rows
        let mut v = Vec::new();
        meow(&mut v, 760.0, 0.4, 0.4);
        tone(&mut v, 0.0, 0.05, 0.0, 0.5);
        meow(&mut v, 880.0, 0.4, 0.4);
        let clear = ld(v).await;

        // four lines: three rising meows
        let mut v = Vec::new();
        meow(&mut v, 700.0, 0.28, 0.4);
        meow(&mut v, 850.0, 0.28, 0.4);
        meow(&mut v, 1000.0, 0.34, 0.4);
        let tetris = ld(v).await;

        let mut v = Vec::new();
        for f in [523.25, 659.25, 783.99, 1046.5] {
            tone(&mut v, f, 0.1, 0.28, 0.25);
        }
        tone(&mut v, 1318.5, 0.3, 0.28, 0.25);
        let level = ld(v).await;

        // sad long meow
        let mut v = Vec::new();
        meow(&mut v, 480.0, 1.2, 0.45);
        let over = ld(v).await;

        // little "mew" when a cat piece appears
        let mut v = Vec::new();
        meow(&mut v, 1300.0, 0.18, 0.3);
        let mew = ld(v).await;

        // explosion
        let mut v = Vec::new();
        noise(&mut v, 0.4, 0.7);
        let mut w = Vec::new();
        sweep(&mut w, 180.0, 35.0, 0.4, 0.5);
        mix(&mut v, &w);
        let boom = ld(v).await;

        let music = ld(build_music()).await;

        Sounds { mv, rot, lock, drop, clear, tetris, level, over, mew, boom, music }
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
    color: Color,
}

struct Boom {
    x: f32,
    y: f32,
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
            play_sound_once(&snd.mew);
        }
        if !self.valid(&self.cur) {
            self.over = true;
        }
    }

    fn burst(&mut self, cx: i32, cy: i32, color: Color, count: usize) {
        let px = BX + (cx as f32 + 0.5) * CELL;
        let py = BY + (cy as f32 + 0.5) * CELL;
        for _ in 0..count {
            let life = rand::gen_range(0.4f32, 0.9);
            self.particles.push(Particle {
                x: px + rand::gen_range(-10.0f32, 10.0),
                y: py + rand::gen_range(-10.0f32, 10.0),
                vx: rand::gen_range(-200.0f32, 200.0),
                vy: rand::gen_range(-320.0f32, -20.0),
                life,
                max: life,
                size: rand::gen_range(3.0f32, 7.0),
                color,
            });
        }
    }

    fn update_fx(&mut self, dt: f32) {
        for p in self.particles.iter_mut() {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.vy += 700.0 * dt;
            p.life -= dt;
        }
        self.particles.retain(|p| p.life > 0.0);
        for b in self.booms.iter_mut() {
            b.t += dt;
        }
        self.booms.retain(|b| b.t < BOOM_DUR);
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
        // the cat itself goes up in fur and sparks
        self.burst(cx, cy, piece_color(CAT), 12);
        self.burst(cx, cy, Color::new(1.0, 0.6, 0.1, 1.0), 10);
        self.burst(cx, cy, Color::new(1.0, 0.95, 0.5, 1.0), 8);
        self.booms.push(Boom {
            x: BX + (cx as f32 + 0.5) * CELL,
            y: BY + (cy as f32 + 0.5) * CELL,
            t: 0.0,
        });
        self.score += destroyed * 10 * (self.level + 1);
        play_sound_once(&snd.boom);
        play_sound_once(&snd.mew);
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
            }
        }
        let rows: Vec<usize> = (0..H)
            .filter(|&r| self.board[r].iter().all(|&c| c != 0))
            .collect();
        if rows.is_empty() {
            play_sound_once(&snd.lock);
            self.spawn(snd);
        } else {
            if rows.len() >= 4 {
                play_sound_once(&snd.tetris);
            } else {
                play_sound_once(&snd.clear);
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
        self.score += [0u32, 40, 100, 300, 1200][n.min(4)] * (self.level + 1);
        for &r in &rows {
            for i in (1..=r).rev() {
                self.board[i] = self.board[i - 1];
            }
            self.board[0] = [0; W];
        }
        self.lines += n as u32;
        self.level = self.start_level + self.lines / 10;
        if self.level > old_level {
            play_sound_once(&snd.level);
        }
        self.spawn(snd);
    }
}

// --------------------------------------------------------------- drawing ----

fn draw_block(px: f32, py: f32, size: f32, c: Color) {
    let b = (size / 10.0).max(2.0);
    draw_rectangle(px, py, size, size, c);
    let light = Color::new((c.r + 0.35).min(1.0), (c.g + 0.35).min(1.0), (c.b + 0.35).min(1.0), 1.0);
    let dark = Color::new(c.r * 0.45, c.g * 0.45, c.b * 0.45, 1.0);
    draw_rectangle(px, py, size, b, light);
    draw_rectangle(px, py, b, size, light);
    draw_rectangle(px, py + size - b, size, b, dark);
    draw_rectangle(px + size - b, py, b, size, dark);
}

// Front-facing cat head; blinks red because it is about to explode.
fn draw_cat_face(px: f32, py: f32, s: f32) {
    let flash = ((get_time() * 6.0) as i32) % 2 == 0;
    let fur = if flash {
        Color::new(1.0, 0.5, 0.4, 1.0)
    } else {
        Color::new(0.78, 0.78, 0.84, 1.0)
    };
    let pink = Color::new(1.0, 0.6, 0.7, 1.0);
    let cx = px + s * 0.5;
    let cy = py + s * 0.58;
    // ears
    draw_triangle(vec2(px + 0.08 * s, py + 0.50 * s), vec2(px + 0.10 * s, py + 0.04 * s), vec2(px + 0.46 * s, py + 0.28 * s), fur);
    draw_triangle(vec2(px + 0.92 * s, py + 0.50 * s), vec2(px + 0.90 * s, py + 0.04 * s), vec2(px + 0.54 * s, py + 0.28 * s), fur);
    draw_triangle(vec2(px + 0.16 * s, py + 0.40 * s), vec2(px + 0.17 * s, py + 0.15 * s), vec2(px + 0.36 * s, py + 0.29 * s), pink);
    draw_triangle(vec2(px + 0.84 * s, py + 0.40 * s), vec2(px + 0.83 * s, py + 0.15 * s), vec2(px + 0.64 * s, py + 0.29 * s), pink);
    // head
    draw_circle(cx, cy, 0.43 * s, fur);
    // eyes
    draw_circle(cx - 0.17 * s, cy - 0.06 * s, 0.09 * s, WHITE);
    draw_circle(cx + 0.17 * s, cy - 0.06 * s, 0.09 * s, WHITE);
    draw_circle(cx - 0.17 * s, cy - 0.05 * s, 0.05 * s, BLACK);
    draw_circle(cx + 0.17 * s, cy - 0.05 * s, 0.05 * s, BLACK);
    // nose and whiskers
    draw_triangle(vec2(cx - 0.05 * s, cy + 0.05 * s), vec2(cx + 0.05 * s, cy + 0.05 * s), vec2(cx, cy + 0.11 * s), pink);
    let wc = Color::new(0.15, 0.15, 0.2, 1.0);
    draw_line(cx - 0.12 * s, cy + 0.10 * s, cx - 0.44 * s, cy + 0.04 * s, 1.0, wc);
    draw_line(cx - 0.12 * s, cy + 0.14 * s, cx - 0.44 * s, cy + 0.18 * s, 1.0, wc);
    draw_line(cx + 0.12 * s, cy + 0.10 * s, cx + 0.44 * s, cy + 0.04 * s, 1.0, wc);
    draw_line(cx + 0.12 * s, cy + 0.14 * s, cx + 0.44 * s, cy + 0.18 * s, 1.0, wc);
}

// One unit of any piece kind (colored block or the cat).
fn draw_unit(px: f32, py: f32, size: f32, kind: usize) {
    if kind == CAT {
        draw_cat_face(px, py, size);
    } else {
        draw_block(px, py, size, piece_color(kind));
    }
}

// Side-view cat that runs along a row and eats the blocks.
fn draw_cat_runner(x: f32, y: f32, s: f32, t: f32) {
    let fur = Color::new(0.95, 0.62, 0.25, 1.0);
    let dark = Color::new(0.70, 0.40, 0.12, 1.0);
    let step = ((t * 14.0) as i32) % 2 == 0;
    let d = if step { 0.04 * s } else { -0.04 * s };
    // tail
    draw_line(x + 0.12 * s, y + 0.5 * s, x - 0.08 * s, y + 0.18 * s, 0.1 * s, fur);
    // legs
    draw_rectangle(x + 0.15 * s + d, y + 0.72 * s, 0.1 * s, 0.26 * s, dark);
    draw_rectangle(x + 0.50 * s - d, y + 0.72 * s, 0.1 * s, 0.26 * s, dark);
    // body
    draw_rectangle(x + 0.10 * s, y + 0.35 * s, 0.55 * s, 0.42 * s, fur);
    draw_circle(x + 0.10 * s, y + 0.56 * s, 0.21 * s, fur);
    // head
    let hx = x + 0.74 * s;
    let hy = y + 0.42 * s;
    draw_circle(hx, hy, 0.26 * s, fur);
    draw_triangle(vec2(hx - 0.20 * s, hy - 0.10 * s), vec2(hx - 0.14 * s, hy - 0.38 * s), vec2(hx, hy - 0.20 * s), fur);
    draw_triangle(vec2(hx + 0.02 * s, hy - 0.20 * s), vec2(hx + 0.18 * s, hy - 0.38 * s), vec2(hx + 0.24 * s, hy - 0.08 * s), fur);
    // eye
    draw_circle(hx + 0.08 * s, hy - 0.06 * s, 0.06 * s, WHITE);
    draw_circle(hx + 0.10 * s, hy - 0.06 * s, 0.03 * s, BLACK);
    // chewing mouth
    if ((t * 10.0) as i32) % 2 == 0 {
        draw_triangle(
            vec2(hx + 0.26 * s, hy),
            vec2(hx + 0.02 * s, hy + 0.08 * s),
            vec2(hx + 0.26 * s, hy + 0.22 * s),
            Color::new(0.5, 0.05, 0.05, 1.0),
        );
    }
    draw_circle(hx + 0.24 * s, hy - 0.02 * s, 0.03 * s, Color::new(1.0, 0.6, 0.7, 1.0));
}

fn center_text(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let d = measure_text(text, None, size as u16, 1.0);
    draw_text(text, cx - d.width / 2.0, y, size, color);
}

fn draw_next(p: &Piece, cx: f32, cy: f32) {
    if p.kind == CAT {
        draw_cat_face(cx - 22.0, cy - 22.0, 44.0);
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
    clear_background(Color::new(0.05, 0.05, 0.09, 1.0));
    center_text("CAT TETRIS", BX + W as f32 * CELL / 2.0, 36.0, 40.0, WHITE);

    // well
    let bw = W as f32 * CELL;
    let bh = H as f32 * CELL;
    draw_rectangle(BX - 6.0, BY - 6.0, bw + 12.0, bh + 12.0, Color::new(0.35, 0.37, 0.5, 1.0));
    draw_rectangle(BX, BY, bw, bh, Color::new(0.02, 0.02, 0.04, 1.0));
    for i in 1..W {
        draw_line(BX + i as f32 * CELL, BY, BX + i as f32 * CELL, BY + bh, 1.0, Color::new(1.0, 1.0, 1.0, 0.05));
    }
    for i in 1..H {
        draw_line(BX, BY + i as f32 * CELL, BX + bw, BY + i as f32 * CELL, 1.0, Color::new(1.0, 1.0, 1.0, 0.05));
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

    // ghost + current piece
    if state != State::Title && game.clearing.is_empty() {
        let g = game.ghost();
        let col = piece_color(game.cur.kind);
        for (x, y) in g.cells() {
            if y >= 0 {
                draw_rectangle_lines(
                    BX + x as f32 * CELL + 1.0,
                    BY + y as f32 * CELL + 1.0,
                    CELL - 2.0,
                    CELL - 2.0,
                    2.0,
                    Color::new(col.r, col.g, col.b, 0.45),
                );
            }
        }
        // show the blast area where the cat will land
        if game.cur.kind == CAT {
            let x0 = (g.x - BOMB_R).max(0);
            let x1 = (g.x + BOMB_R).min(W as i32 - 1);
            let y0 = (g.y - BOMB_R).max(0);
            let y1 = (g.y + BOMB_R).min(H as i32 - 1);
            draw_rectangle(
                BX + x0 as f32 * CELL,
                BY + y0 as f32 * CELL,
                (x1 - x0 + 1) as f32 * CELL,
                (y1 - y0 + 1) as f32 * CELL,
                Color::new(1.0, 0.2, 0.1, 0.12),
            );
            draw_rectangle_lines(
                BX + x0 as f32 * CELL,
                BY + y0 as f32 * CELL,
                (x1 - x0 + 1) as f32 * CELL,
                (y1 - y0 + 1) as f32 * CELL,
                2.0,
                Color::new(1.0, 0.3, 0.2, 0.7),
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
        for &r in &game.clearing {
            draw_cat_runner(x, BY + r as f32 * CELL, CELL, get_time() as f32);
        }
    }

    // explosions and crumbs
    for b in &game.booms {
        let p = b.t / BOOM_DUR;
        let radius = p * (BOMB_R as f32 + 0.8) * CELL;
        let a = 1.0 - p;
        draw_circle(b.x, b.y, radius, Color::new(1.0, 0.55, 0.1, 0.5 * a));
        draw_circle(b.x, b.y, radius * 0.6, Color::new(1.0, 0.95, 0.6, 0.7 * a));
        draw_circle_lines(b.x, b.y, radius, 3.0, Color::new(1.0, 1.0, 1.0, a));
    }
    for p in &game.particles {
        let a = (p.life / p.max).clamp(0.0, 1.0);
        draw_rectangle(p.x, p.y, p.size, p.size, Color::new(p.color.r, p.color.g, p.color.b, a));
    }

    // side panel
    let label = Color::new(0.6, 0.65, 0.85, 1.0);
    let pw = 190.0;
    draw_text("NEXT", PANEL_X, 70.0, 26.0, label);
    draw_rectangle(PANEL_X, 80.0, pw, 110.0, Color::new(0.02, 0.02, 0.04, 1.0));
    draw_rectangle_lines(PANEL_X, 80.0, pw, 110.0, 2.0, Color::new(0.35, 0.37, 0.5, 1.0));
    if state != State::Title {
        draw_next(&game.next, PANEL_X + pw / 2.0, 135.0);
    }

    let shown_level = if state == State::Title { start_level } else { game.level };
    let stats: [(&str, String); 4] = [
        ("SCORE", format!("{}", game.score)),
        ("LINES", format!("{}", game.lines)),
        ("LEVEL", format!("{}", shown_level)),
        ("HIGH SCORE", format!("{}", hi)),
    ];
    let mut y = 235.0;
    for (name, val) in stats.iter() {
        draw_text(name, PANEL_X, y, 24.0, label);
        draw_text(val, PANEL_X, y + 30.0, 34.0, WHITE);
        y += 80.0;
    }

    draw_text("ARROWS  move / soft drop", PANEL_X, 580.0, 18.0, GRAY);
    draw_text("UP / X  rotate    Z  rotate back", PANEL_X, 602.0, 18.0, GRAY);
    draw_text("SPACE  hard drop   P  pause", PANEL_X, 624.0, 18.0, GRAY);
    draw_text(
        if music_on { "M  music: ON" } else { "M  music: OFF" },
        PANEL_X,
        646.0,
        18.0,
        GRAY,
    );

    // overlays
    let cx = BX + bw / 2.0;
    let overlay = || draw_rectangle(BX, BY, bw, bh, Color::new(0.0, 0.0, 0.0, 0.72));
    match state {
        State::Title => {
            overlay();
            center_text("CAT TETRIS", cx, 250.0, 50.0, Color::new(0.97, 0.62, 0.25, 1.0));
            draw_cat_face(cx - 25.0, 270.0, 50.0);
            center_text("Start level", cx, 360.0, 26.0, WHITE);
            center_text(&format!("< {} >", start_level), cx, 405.0, 48.0, Color::new(0.2, 0.8, 0.9, 1.0));
            center_text("UP / DOWN to change", cx, 435.0, 20.0, GRAY);
            center_text("Press ENTER to play", cx, 495.0, 28.0, WHITE);
            center_text("Cat pieces explode when they land!", cx, 525.0, 18.0, GRAY);
            center_text("ESC to quit", cx, 550.0, 20.0, GRAY);
        }
        State::Paused => {
            overlay();
            center_text("PAUSED", cx, BY + bh / 2.0, 50.0, WHITE);
            center_text("Press P to resume", cx, BY + bh / 2.0 + 35.0, 24.0, GRAY);
        }
        State::Over => {
            overlay();
            center_text("GAME OVER", cx, BY + bh / 2.0 - 20.0, 50.0, Color::new(0.95, 0.25, 0.25, 1.0));
            center_text(&format!("Score {}", game.score), cx, BY + bh / 2.0 + 25.0, 30.0, WHITE);
            center_text("Press ENTER", cx, BY + bh / 2.0 + 65.0, 24.0, GRAY);
        }
        State::Playing => {}
    }
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

fn start_music(snd: &Sounds) {
    play_sound(&snd.music, PlaySoundParams { looped: true, volume: 0.35 });
}

#[macroquad::main(conf)]
async fn main() {
    rand::srand((get_time() * 1_000_000.0) as u64 ^ 0x9E37_79B9);
    let snd = Sounds::load().await;

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
                    start_music(&snd);
                }
            } else {
                stop_sound(&snd.music);
            }
        }

        match state {
            State::Title => {
                if is_key_pressed(KeyCode::Up) && start_level < 19 {
                    start_level += 1;
                    play_sound_once(&snd.mv);
                }
                if is_key_pressed(KeyCode::Down) && start_level > 0 {
                    start_level -= 1;
                    play_sound_once(&snd.mv);
                }
                if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                    game = Game::new(start_level);
                    state = State::Playing;
                    if music_on {
                        start_music(&snd);
                    }
                }
            }
            State::Paused => {
                if is_key_pressed(KeyCode::P) {
                    state = State::Playing;
                    if music_on {
                        start_music(&snd);
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
                    stop_sound(&snd.music);
                } else if !game.clearing.is_empty() {
                    game.update_clear(dt, &snd);
                } else {
                    if auto_repeat(KeyCode::Left, &mut game.tl, dt, 0.17, 0.05) && game.try_move(-1, 0) {
                        play_sound_once(&snd.mv);
                    }
                    if auto_repeat(KeyCode::Right, &mut game.tr, dt, 0.17, 0.05) && game.try_move(1, 0) {
                        play_sound_once(&snd.mv);
                    }
                    if (is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::X)) && game.try_rotate(true) {
                        play_sound_once(&snd.rot);
                    }
                    if (is_key_pressed(KeyCode::Z) || is_key_pressed(KeyCode::LeftControl))
                        && game.try_rotate(false)
                    {
                        play_sound_once(&snd.rot);
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
                        play_sound_once(&snd.drop);
                        game.lock(&snd);
                    }
                }

                if game.over {
                    state = State::Over;
                    hi = hi.max(game.score);
                    stop_sound(&snd.music);
                    play_sound_once(&snd.over);
                }
            }
        }

        game.update_fx(dt);
        hi = hi.max(game.score);
        draw_game(&game, state, hi, start_level, music_on);
        next_frame().await;
    }
}
