//! Show the Lyrics Horizon para Horizon.
//!
//! Plays the songs the player puts in the "Mi Música" resource pack, with their synced lyrics
//! floating on screen (LRC files), a now-playing widget and keys to control it.
//!
//! How songs are found:
//! - The mod's `sounds.json` names 100 slots, `musica:cancion.1` .. `musica:cancion.100`,
//!   each the file `assets/musica/sounds/canciones/<n>.ogg` of any selected resource pack.
//!   Slots with no file are dropped by the game, as vanilla drops missing sounds.
//! - A client reload listener lists which `<n>.ogg` exist, so only real songs are played.
//! - `assets/musica/canciones/nombres.txt` (`<n>=Artista - Título`) names them.
//! - `assets/musica/letras/<n>.lrc` holds a song's synced lyrics (`[mm:ss.xx]line`).
//!
//! Keys (Options > Controls, "Show the Lyrics Horizon"): M play/stop, N next, B previous,
//! K show/hide lyrics.
//!
//! License: MIT (see LICENSE).
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mod_api::*;

const ID: &str = "musica";
/// Song slots declared in sounds.json.
const SLOTS: u32 = 100;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- lyrics

/// One timed line of an LRC file.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// When it starts, in milliseconds from the song's start.
    pub at: i64,
    pub text: String,
    /// When each word is sung, in ms from the line's start (enhanced LRC `<mm:ss.xx>` tags);
    /// empty when the file only times lines.
    pub words: Vec<i64>,
}

/// A song's lyrics and the tags its LRC file gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub lines: Vec<Line>,
    /// The file's own `[offset:]`, in ms (already applied to the lines).
    pub offset: i64,
}

/// `mm:ss.xx` (or `mm:ss`, `mm:ss:xx`) in milliseconds.
fn parse_time(t: &str) -> Option<i64> {
    let (m, rest) = t.split_once(':')?;
    let m: i64 = m.trim().parse().ok()?;
    let rest = rest.replace(':', ".");
    let (s, frac) = rest.split_once('.').unwrap_or((&rest, ""));
    let s: i64 = s.trim().parse().ok()?;
    let ms = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take(3).collect();
        let v: i64 = digits.parse().ok()?;
        match digits.len() {
            1 => v * 100,
            2 => v * 10,
            _ => v,
        }
    };
    Some(m * 60_000 + s * 1000 + ms)
}

/// Reads an LRC file: `[ti:]`, `[ar:]`, `[offset:]` tags and `[mm:ss.xx]` lines (several stamps
/// on one line repeat it). Lines come out in time order.
pub fn parse_lrc(text: &str) -> Lyrics {
    let mut lyrics = Lyrics::default();
    let mut offset = 0i64;
    for raw in text.lines() {
        let mut rest = raw.trim().trim_start_matches('\u{feff}');
        let mut stamps = Vec::new();
        while let Some(body) = rest.strip_prefix('[') {
            let Some(end) = body.find(']') else { break };
            let tag = &body[..end];
            rest = &body[end + 1..];
            if let Some(ms) = parse_time(tag) {
                stamps.push(ms);
            } else if let Some((k, v)) = tag.split_once(':') {
                let v = v.trim().to_string();
                match k.trim().to_ascii_lowercase().as_str() {
                    "ti" if !v.is_empty() => lyrics.title = Some(v),
                    "ar" if !v.is_empty() => lyrics.artist = Some(v),
                    "offset" => offset = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        let (line, times) = word_times(rest);
        for at in stamps {
            let words = if times.is_empty() { Vec::new() } else { times.iter().map(|t| t - at).collect() };
            lyrics.lines.push(Line { at, text: line.clone(), words });
        }
    }
    // A positive offset shows lyrics sooner, as LRC players read it.
    for l in &mut lyrics.lines {
        l.at -= offset;
    }
    lyrics.offset = offset;
    lyrics.lines.sort_by_key(|l| l.at);
    lyrics
}

/// A line's text without its `<mm:ss.xx>` word tags, and each word's time when every word has one.
fn word_times(rest: &str) -> (String, Vec<i64>) {
    if !rest.contains('<') {
        return (rest.trim().to_string(), Vec::new());
    }
    let mut text = String::new();
    let mut times = Vec::new();
    let mut pending: Option<i64> = None;
    let mut in_word = false;
    let mut s = rest;
    let mut all = true;
    while !s.is_empty() {
        if let Some(body) = s.strip_prefix('<') {
            if let Some(end) = body.find('>') {
                if let Some(ms) = parse_time(&body[..end]) {
                    pending = Some(ms);
                    s = &body[end + 1..];
                    continue;
                }
            }
        }
        let c = s.chars().next().unwrap_or(' ');
        s = &s[c.len_utf8()..];
        if c.is_whitespace() {
            in_word = false;
            if !text.ends_with(' ') && !text.is_empty() {
                text.push(' ');
            }
            continue;
        }
        if !in_word {
            in_word = true;
            match pending.take().or(times.last().copied()) {
                Some(t) => times.push(t),
                None => all = false,
            }
        }
        text.push(c);
    }
    let text = text.trim().to_string();
    if !all || times.len() != text.split_whitespace().count() {
        times.clear();
    }
    (text, times)
}

/// Lines this close together are sung at the same time (a duet, a second voice).
const TOGETHER_MS: i64 = 40;

impl Lyrics {
    /// The index of the line showing at `ms`, if one has started.
    pub fn current(&self, ms: i64) -> Option<usize> {
        let n = self.lines.partition_point(|l| l.at <= ms);
        n.checked_sub(1)
    }

    /// The first and last line sung together with line `i` (same time stamp).
    pub fn group(&self, i: usize) -> (usize, usize) {
        let at = self.lines[i].at;
        let mut a = i;
        while a > 0 && (at - self.lines[a - 1].at).abs() <= TOGETHER_MS && !self.lines[a - 1].text.trim().is_empty() {
            a -= 1;
        }
        let mut b = i;
        while b + 1 < self.lines.len() && (self.lines[b + 1].at - at).abs() <= TOGETHER_MS && !self.lines[b + 1].text.trim().is_empty() {
            b += 1;
        }
        (a, b)
    }

    /// When the line after `i`'s group starts.
    pub fn next_at(&self, i: usize) -> Option<i64> {
        let at = self.lines[i].at;
        self.lines[i + 1..].iter().find(|l| l.at - at > TOGETHER_MS).map(|l| l.at)
    }
}

/// How many syllables a word has (vowel groups; each CJK character is one).
pub fn syllables(w: &str) -> usize {
    let mut n = 0;
    let mut in_vowel = false;
    for c in w.chars().flat_map(|c| c.to_lowercase()) {
        let wide = ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c) || ('\u{ac00}'..='\u{d7af}').contains(&c);
        if wide {
            n += 1;
            in_vowel = false;
            continue;
        }
        let v = "aeiouyáéíóúüàèìòùâêîôûäëïöãõ".contains(c);
        if v && !in_vowel {
            n += 1;
        }
        in_vowel = v;
    }
    n.max(1)
}

/// When each word of a line without word times is sung, in ms from the line's start: the
/// line's singing time shared out by syllables (a pause after a comma), each word then moved
/// onto the nearest jump in the music's loudness, where a syllable usually lands.
pub fn estimate_words(text: &str, span: i64, at: i64, energy: Option<&[u8]>) -> Vec<i64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let weights: Vec<f32> = words
        .iter()
        .map(|w| syllables(w) as f32 + if w.ends_with([',', '.', ';', '?', '!', '…', '、', '。']) { 0.9 } else { 0.25 })
        .collect();
    let total: f32 = weights.iter().sum::<f32>().max(0.1);
    let mut out = Vec::with_capacity(words.len());
    let mut acc = 0.0f32;
    for w in &weights {
        out.push((span as f32 * 0.94 * acc / total) as i64);
        acc += w;
    }
    if let Some(e) = energy.filter(|e| e.len() > 4) {
        for k in 1..out.len() {
            let t = at + out[k];
            if t < 100 {
                continue;
            }
            let f = (t / 50) as usize;
            let mut best: Option<(i32, usize)> = None;
            for g in f.saturating_sub(2)..=(f + 2).min(e.len() - 1) {
                if g == 0 {
                    continue;
                }
                let rise = e[g] as i32 - e[g - 1] as i32;
                let score = rise * 4 - (g as i32 - f as i32).abs();
                if rise >= 2 && best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, g));
                }
            }
            if let Some((_, g)) = best {
                let lo = out[k - 1] + 80;
                let hi = out.get(k + 1).map(|n| n - 80).unwrap_or(span);
                if lo <= hi {
                    out[k] = (g as i64 * 50 - at).clamp(lo, hi);
                }
            }
        }
    }
    for k in 1..out.len() {
        if out[k] < out[k - 1] {
            out[k] = out[k - 1];
        }
    }
    out
}

/// Word times for line `i`: the file's own, or an estimate from the syllables and the beat.
pub fn word_timing(lyrics: &Lyrics, i: usize, energy: Option<&Rhythm>) -> Vec<i64> {
    let line = &lyrics.lines[i];
    if !line.words.is_empty() {
        return line.words.clone();
    }
    estimate_words(&line.text, sung_span(lyrics, i), line.at, energy.map(|r| r.level.as_slice()))
}

// ---------------------------------------------------------------- library

/// How the song was identified, from `info/<n>.txt` (written by AGREGAR / PREPARAR_CANCIONES).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongInfo {
    /// Where the lyrics came from: lrclib, netease, megalobiz, youtube, ia, manual, "tu .lrc"…
    pub source: String,
    /// 0..100 (100: you chose or wrote it); -1 when unknown.
    pub confidence: i32,
    /// palabra, linea, aproximada, instrumental, ninguna.
    pub kind: String,
    /// auto, revisar, manual, bloqueada, ninguna.
    pub state: String,
    pub locked: bool,
    /// "Artista - Título" as identified.
    pub identified: String,
}

pub fn parse_info(text: &str) -> SongInfo {
    let mut i = SongInfo { confidence: -1, ..Default::default() };
    for l in text.lines() {
        let Some((k, v)) = l.trim().trim_start_matches('\u{feff}').split_once('=') else { continue };
        let v = v.trim().to_string();
        match k.trim() {
            "fuente" => i.source = v,
            "confianza" => i.confidence = v.parse().unwrap_or(-1),
            "tipo" => i.kind = v,
            "estado" => i.state = v,
            "bloqueada" => i.locked = v == "1",
            "identificada" => i.identified = v,
            _ => {}
        }
    }
    i
}

impl SongInfo {
    pub fn instrumental(&self) -> bool {
        self.kind == "instrumental"
    }
    pub fn mine(&self) -> bool {
        self.state == "manual" || self.confidence >= 100
    }
    /// Needs a look in PREPARAR_CANCIONES: little confidence or nothing found.
    pub fn to_check(&self) -> bool {
        !self.instrumental() && !self.mine() && (self.kind == "ninguna" || (0..70).contains(&self.confidence))
    }
    /// The colour of its confidence.
    pub fn colour(&self) -> u32 {
        if self.instrumental() {
            0x6EB4FF
        } else if self.kind == "ninguna" {
            0xFF6E6E
        } else if self.mine() || self.confidence >= 85 {
            0x78E68C
        } else if self.confidence >= 70 {
            0xC8E678
        } else if self.confidence >= 0 {
            0xFFD25A
        } else {
            0xAAAAAA
        }
    }
    /// A few words: "lrclib · 92%", "tuya", "instrumental".
    pub fn short(&self) -> String {
        if self.instrumental() {
            return "instrumental".into();
        }
        if self.kind == "ninguna" {
            return "sin letra".into();
        }
        if self.mine() {
            return format!("{} · tuya", self.source);
        }
        if self.confidence >= 0 {
            format!("{} · {}%", self.source, self.confidence)
        } else {
            self.source.clone()
        }
    }
    pub fn verdict(&self) -> &'static str {
        if self.instrumental() {
            "instrumental"
        } else if self.mine() {
            "elegida por ti"
        } else if self.kind == "ninguna" {
            "sin letra"
        } else if self.confidence >= 85 {
            "identificada"
        } else if self.confidence >= 70 {
            "probable"
        } else if self.confidence >= 0 {
            "por revisar"
        } else {
            "sin datos"
        }
    }
}

/// A song the packs supply.
#[derive(Clone, Debug, Default)]
pub struct Song {
    pub slot: u32,
    pub name: String,
    pub lyrics: Option<Lyrics>,
    pub duration: Option<i64>,
    pub energy: Option<Arc<Rhythm>>,
    pub info: Option<SongInfo>,
}

impl Song {
    pub fn instrumental(&self) -> bool {
        self.info.as_ref().is_some_and(|i| i.instrumental())
    }
}

/// What the last resource load found, published by the reload listeners.
#[derive(Default)]
struct Library {
    slots: Vec<u32>,
    names: BTreeMap<u32, String>,
    lyrics: BTreeMap<u32, Lyrics>,
    /// Song lengths in ms, read from the .ogg files.
    durations: BTreeMap<u32, i64>,
    /// Loudness per 50 ms, from `ritmo/<n>.txt`.
    energy: BTreeMap<u32, Arc<Rhythm>>,
    /// How each song was identified.
    info: BTreeMap<u32, SongInfo>,
}

static LIBRARY: Mutex<Library> = Mutex::new(Library {
    slots: Vec::new(),
    names: BTreeMap::new(),
    lyrics: BTreeMap::new(),
    durations: BTreeMap::new(),
    energy: BTreeMap::new(),
    info: BTreeMap::new(),
});

/// An Ogg Vorbis file's length in ms: the last page's granule position over the sample rate.
pub fn ogg_duration_ms(b: &[u8]) -> Option<i64> {
    let head = &b[..b.len().min(8192)];
    let id = head.windows(7).position(|w| w == b"\x01vorbis")?;
    let rate = u32::from_le_bytes(b.get(id + 12..id + 16)?.try_into().ok()?) as u64;
    let from = b.len().saturating_sub(65536);
    let last = from + b[from..].windows(4).rposition(|w| w == b"OggS")?;
    let granule = u64::from_le_bytes(b.get(last + 6..last + 14)?.try_into().ok()?);
    (rate > 0 && granule != u64::MAX).then(|| (granule * 1000 / rate) as i64)
}

/// `musica:12` (from `canciones/12.ogg`) is slot 12.
fn slot_of(id: &Identifier) -> Option<u32> {
    if id.namespace() != ID {
        return None;
    }
    let n: u32 = id.path().parse().ok()?;
    (1..=SLOTS).contains(&n).then_some(n)
}

/// `nombres.txt`: `12=Artista - Título` per line; `#` starts a comment.
pub fn parse_names(text: &str) -> BTreeMap<u32, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim().trim_start_matches('\u{feff}');
            if l.starts_with('#') {
                return None;
            }
            let (n, name) = l.split_once('=')?;
            let n: u32 = n.trim().parse().ok()?;
            let name = name.trim();
            (!name.is_empty()).then(|| (n, name.to_string()))
        })
        .collect()
}

struct SongFiles;
impl ClientReloadListener for SongFiles {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut slots: Vec<u32> = cx.list().filter_map(slot_of).collect();
        slots.sort_unstable();
        slots.dedup();
        let mut durations = BTreeMap::new();
        for id in cx.list() {
            if let (Some(n), Some(bytes)) = (slot_of(id), cx.read(id)) {
                if let Some(ms) = ogg_duration_ms(bytes) {
                    durations.insert(n, ms);
                }
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            let mut lib = lock(&LIBRARY);
            lib.slots = slots;
            lib.durations = durations;
        }))
    }
}

struct NameFiles;
impl ClientReloadListener for NameFiles {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut names = BTreeMap::new();
        for id in cx.list() {
            if id.namespace() == ID && id.path() == "nombres" {
                if let Some(bytes) = cx.read(id) {
                    names = parse_names(&String::from_utf8_lossy(bytes));
                }
            }
        }
        Ok(<dyn ReloadApply>::new(move || lock(&LIBRARY).names = names))
    }
}

struct LyricFiles;
impl ClientReloadListener for LyricFiles {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut lyrics = BTreeMap::new();
        for id in cx.list() {
            if let (Some(n), Some(bytes)) = (slot_of(id), cx.read(id)) {
                let parsed = parse_lrc(&String::from_utf8_lossy(bytes));
                if !parsed.lines.is_empty() || parsed.title.is_some() {
                    lyrics.insert(n, parsed);
                }
            }
        }
        Ok(<dyn ReloadApply>::new(move || lock(&LIBRARY).lyrics = lyrics))
    }
}

struct InfoFiles;
impl ClientReloadListener for InfoFiles {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut info = BTreeMap::new();
        for id in cx.list() {
            if let (Some(n), Some(bytes)) = (slot_of(id), cx.read(id)) {
                info.insert(n, parse_info(&String::from_utf8_lossy(bytes)));
            }
        }
        Ok(<dyn ReloadApply>::new(move || lock(&LIBRARY).info = info))
    }
}

struct RhythmFiles;
impl ClientReloadListener for RhythmFiles {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut energy = BTreeMap::new();
        for id in cx.list() {
            if let (Some(n), Some(bytes)) = (slot_of(id), cx.read(id)) {
                let e = Rhythm::parse(bytes);
                if !e.level.is_empty() {
                    energy.insert(n, Arc::new(e));
                }
            }
        }
        Ok(<dyn ReloadApply>::new(move || lock(&LIBRARY).energy = energy))
    }
}

fn songs() -> Vec<Song> {
    let lib = lock(&LIBRARY);
    lib.slots
        .iter()
        .map(|&slot| {
            let lyrics = lib.lyrics.get(&slot).cloned();
            let name = lib.names.get(&slot).cloned().unwrap_or_else(|| {
                match lyrics.as_ref().map(|l| (l.artist.clone(), l.title.clone())) {
                    Some((Some(a), Some(t))) => format!("{a} - {t}"),
                    Some((None, Some(t))) => t,
                    _ => format!("Canción {slot}"),
                }
            });
            let duration = lib.durations.get(&slot).copied();
            let energy = lib.energy.get(&slot).cloned();
            let info = lib.info.get(&slot).cloned();
            // An instrumental keeps no lyrics, even if an old .lrc is left.
            let lyrics = if info.as_ref().is_some_and(|i| i.instrumental()) { None } else { lyrics };
            Song { slot, name, lyrics, duration, energy, info }
        })
        .collect()
}

// ---------------------------------------------------------------- settings

pub fn settings() -> Vec<SettingSpec> {
    let k = |key: &str| format!("musica.setting.{key}");
    let tip = |key: &str| format!("musica.setting.{key}.tooltip");
    vec![
        SettingSpec::range("volume", k("volume"), 0.0, 100.0, 5.0, 100.0).tooltip(tip("volume")),
        SettingSpec::toggle("lyrics", k("lyrics"), true).tooltip(tip("lyrics")),
        SettingSpec::choice("lyrics_mode", k("lyrics_mode"), [k("mode.world"), k("mode.screen")], 0)
            .tooltip(tip("lyrics_mode")),
        SettingSpec::choice(
            "lyrics_style",
            k("lyrics_style"),
            STYLE_KEYS.iter().map(|s| k(&format!("style.{s}"))).collect::<Vec<_>>(),
            1,
        )
        .tooltip(tip("lyrics_style")),
        SettingSpec::range("rhythm", k("rhythm"), 0.0, 100.0, 10.0, 60.0).tooltip(tip("rhythm")),
        SettingSpec::choice("rhythm_fx", k("rhythm_fx"), FX_KEYS.iter().map(|s| k(&format!("fx.{s}"))).collect::<Vec<_>>(), 14)
            .tooltip(tip("rhythm_fx")),
        SettingSpec::choice(
            "rhythm_source",
            k("rhythm_source"),
            SOURCE_KEYS.iter().map(|s| k(&format!("source.{s}"))).collect::<Vec<_>>(),
            0,
        )
        .tooltip(tip("rhythm_source")),
        SettingSpec::choice("visualizer", k("visualizer"), [k("vis.off"), k("vis.top"), k("vis.bottom")], 2)
            .tooltip(tip("visualizer")),
        SettingSpec::toggle("title_no_lyrics", k("title_no_lyrics"), true).tooltip(tip("title_no_lyrics")),
        SettingSpec::choice("lyrics_font", k("lyrics_font"), FONT_KEYS.iter().map(|s| k(&format!("font.{s}"))).collect::<Vec<_>>(), 0)
            .tooltip(tip("lyrics_font")),
        SettingSpec::choice("lyrics_ttf", k("lyrics_ttf"), TTF_KEYS.iter().map(|s| k(&format!("ttf.{s}"))).collect::<Vec<_>>(), 0)
            .tooltip(tip("lyrics_ttf")),
        SettingSpec::toggle("banner", k("banner"), true).tooltip(tip("banner")),
        SettingSpec::range("lyrics_offset", k("lyrics_offset"), -2000.0, 2000.0, 50.0, 0.0)
            .tooltip(tip("lyrics_offset")),
        SettingSpec::range("lyrics_distance", k("lyrics_distance"), 2.0, 12.0, 0.5, 5.0)
            .tooltip(tip("lyrics_distance")),
        SettingSpec::range("lyrics_lines", k("lyrics_lines"), 1.0, 6.0, 1.0, 3.0).tooltip(tip("lyrics_lines")),
        SettingSpec::choice(
            "lyrics_place",
            k("lyrics_place"),
            [k("place.top"), k("place.middle"), k("place.bottom")],
            2,
        )
        .tooltip(tip("lyrics_place")),
        SettingSpec::range("lyrics_size", k("lyrics_size"), 1.0, 4.0, 0.5, 2.0).tooltip(tip("lyrics_size")),
        SettingSpec::choice(
            "lyrics_color",
            k("lyrics_color"),
            COLOR_KEYS.iter().map(|s| k(&format!("color.{s}"))).collect::<Vec<_>>(),
            0,
        )
        .tooltip(tip("lyrics_color")),
        SettingSpec::toggle("lyrics_box", k("lyrics_box"), false).tooltip(tip("lyrics_box")),
        SettingSpec::toggle("lyrics_context", k("lyrics_context"), true).tooltip(tip("lyrics_context")),
        SettingSpec::toggle("widget", k("widget"), true).tooltip(tip("widget")),
        SettingSpec::choice("widget_corner", k("widget_corner"), [k("corner.left"), k("corner.right")], 0)
            .tooltip(tip("widget_corner")),
        SettingSpec::toggle("shuffle", k("shuffle"), false).tooltip(tip("shuffle")),
        SettingSpec::choice(
            "repeat",
            k("repeat"),
            [k("repeat.all"), k("repeat.one"), k("repeat.off")],
            0,
        )
        .tooltip(tip("repeat")),
        SettingSpec::toggle("mute_game_music", k("mute_game_music"), true).tooltip(tip("mute_game_music")),
        SettingSpec::range("lyrics_opacity", k("lyrics_opacity"), 10.0, 100.0, 10.0, 100.0).tooltip(tip("lyrics_opacity")),
        SettingSpec::toggle("crossfade", k("crossfade"), true).tooltip(tip("crossfade")),
        SettingSpec::choice("world_layout", k("world_layout"), LAYOUT_KEYS.iter().map(|s| k(&format!("layout.{s}"))).collect::<Vec<_>>(), 0)
            .tooltip(tip("world_layout")),
        SettingSpec::choice("world_facing", k("world_facing"), FACING_KEYS.iter().map(|s| k(&format!("facing.{s}"))).collect::<Vec<_>>(), 0)
            .tooltip(tip("world_facing")),
        SettingSpec::range("camera_sway", k("camera_sway"), 0.0, 10.0, 1.0, 0.0).tooltip(tip("camera_sway")),
        SettingSpec::toggle("dodge_f3", k("dodge_f3"), true).tooltip(tip("dodge_f3")),
    ]
}

/// Where the floating lines appear around you.
const LAYOUT_KEYS: [&str; 7] = ["front", "left", "right", "ring", "above", "map", "scatter"];
pub const LAYOUT_NAMES: [&str; 7] = ["De frente", "A la izquierda", "A la derecha", "En anillo", "Arriba", "En el mapa", "Por todas partes"];
/// Which way they look: at you, or fixed in the world like a sign (seen from the side as you move).
const FACING_KEYS: [&str; 3] = ["you", "fixed", "angled"];
pub const FACING_NAMES: [&str; 3] = ["Te miran", "Fijas como cartel", "De lado"];
/// The menu's colours.
pub const THEME_NAMES: [&str; 7] = ["Morado", "Océano", "Atardecer", "Bosque", "Carbón", "Caramelo", "Dorado"];

const STYLE_KEYS: [&str; 34] = [
    "classic", "karaoke", "bounce", "wave", "neon", "cinema", "typewriter", "fire", "glitch", "rainbow", "outline", "giant",
    "heartbeat", "rain", "disco", "echo", "matrix", "explosion", "retro", "shine", "swing", "hologram", "longshadow",
    "gradient", "bubble", "ice", "gold", "ink", "quake", "spiral", "flyin", "ghost", "sticker", "pulse",
];
pub const STYLE_NAMES: [&str; 34] = [
    "Clásico", "Karaoke", "Rebote", "Ola", "Neón", "Cine", "Máquina de escribir", "Fuego", "Glitch", "Arcoíris", "Contorno",
    "Gigante", "Latido", "Lluvia", "Disco", "Eco", "Matrix", "Explosión", "Retro", "Brillo", "Columpio", "Holograma",
    "Sombra larga", "Degradado", "Burbujas", "Hielo", "Oro", "Tinta", "Terremoto", "Espiral", "Entrada", "Fantasma",
    "Pegatina", "Pulso",
];
const COLOR_KEYS: [&str; 15] = ["white", "gold", "aqua", "pink", "green", "red", "purple", "orange", "lime", "coral", "lavender", "mint", "sky", "magenta", "rainbow"];
pub const COLOR_NAMES: [&str; 15] = ["Blanco", "Dorado", "Celeste", "Rosa", "Verde", "Rojo", "Morado", "Naranja", "Lima", "Coral", "Lavanda", "Menta", "Cielo", "Magenta", "Arcoíris"];
/// Plain 0xRRGGBB colours of the "lyrics_color" choice; the last choice is the rainbow.
const COLORS: [u32; 14] = [0xFFFFFF, 0xFFD24A, 0x55FFFF, 0xFF7AD9, 0x6CFF6C, 0xFF5555, 0xC28BFF, 0xFFA040, 0xC6FF3A, 0xFF8A70, 0xD6B8FF, 0x8CFFD0, 0x8CC8FF, 0xFF40C0];

fn hsv(h: f32) -> u32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    let c = |v: f32| ((v * 0.55 + 0.45) * 255.0) as u32; // pastel, readable
    (c(r) << 16) | (c(g) << 8) | c(b)
}

fn argb(rgb: u32, alpha: f32) -> u32 {
    // Vanilla draws nothing below alpha 4; keep a fading line visible to the end.
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
    (a.max(5) << 24) | (rgb & 0xFF_FFFF)
}

/// Mixes two 0xRRGGBB colours, `t` of the way from `a` to `b`.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| {
        let x = ((a >> s) & 0xFF) as f32;
        let y = ((b >> s) & 0xFF) as f32;
        ((x + (y - x) * t) as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

// ---------------------------------------------------------------- player

#[derive(Default)]
struct Playing {
    song: Song,
    sound: Option<SoundRef>,
    /// Ticks spent waiting for the sound to start.
    ticks: u64,
    /// `host.millis()` when the sound started playing.
    start: Option<u64>,
    /// Milliseconds the game spent paused since then (the sound waits with it).
    paused: u64,
    /// When the current pause began.
    pause_at: Option<u64>,
    /// Turned down while the pause menu shows in multiplayer (the game doesn't pause there).
    quiet: bool,
    /// A style picked at random for this song.
    style: Option<usize>,
    /// It started quiet and turns up once it plays (smooth change of song).
    fade_in: bool,
    /// The next song already started over its last seconds.
    ending: bool,
}

impl Playing {
    /// How far into the song we are, by the game's clock, pauses left out.
    fn ms(&self, now: u64) -> i64 {
        let Some(start) = self.start else { return 0 };
        let end = self.pause_at.unwrap_or(now);
        end.saturating_sub(start).saturating_sub(self.paused) as i64
    }
}

#[derive(Default)]
struct Player {
    now: Option<Playing>,
    /// Position in the play order.
    index: usize,
    order: Vec<u32>,
    seed: u64,
    /// A message shown briefly ("No hay canciones…"), with ticks left.
    toast: Option<(String, u32)>,
    /// Client ticks, for the toast and randomness.
    tick: u64,
    /// Lyric lines floating in the world.
    floating: Vec<Floating>,
    /// The lyric line last put in the world, and of which song.
    placed: Option<(u32, usize)>,
    spawned: u32,
    /// `host.millis()` when the pause menu was last drawn.
    pause_menu_seen: u64,
    /// Songs fading out after a change, stopped for good when the menu closes the game.
    old: Vec<(SoundRef, u64)>,
    /// Lyric delay for one song, changed live with the , and . keys (ms, + shows sooner).
    pub song_offset: BTreeMap<u32, i64>,
}

/// How long the change of song takes (ticks): the old one fades out as the new one fades in.
const FADE_OUT_TICKS: u32 = 16;
const FADE_IN_TICKS: u32 = 14;
/// The next song starts this long before the end of the one playing.
const CROSSFADE_MS: i64 = 1600;

/// A lyric line hanging in the world where it appeared.
struct Floating {
    line: usize,
    pos: [f64; 3],
    /// Song time (ms) it appeared at.
    born: i64,
    /// Song time it stopped being the current line.
    replaced: Option<i64>,
    /// Song time it fell out of the lines kept (it fades from then).
    old_since: Option<i64>,
    hue: f32,
    /// Fixed in the world: the direction the line runs along (horizontal, unit length).
    along: Option<[f64; 3]>,
}

#[derive(Default)]
struct Musica {
    player: Mutex<Player>,
    menu: Mutex<menu::Menu>,
}

const NO_SONGS: &str = "No hay canciones: pon tus .ogg en el paquete Mi Música y pulsa F3+T";

impl Player {
    fn random(&mut self) -> u64 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407 ^ self.tick);
        self.seed >> 33
    }

    /// The play order over the songs found, shuffled when asked.
    fn rebuild_order(&mut self, all: &[Song], shuffle: bool) {
        let current = self.now.as_ref().map(|p| p.song.slot);
        self.order = all.iter().map(|s| s.slot).collect();
        if shuffle {
            for i in (1..self.order.len()).rev() {
                let j = (self.random() % (i as u64 + 1)) as usize;
                self.order.swap(i, j);
            }
        }
        if let Some(slot) = current {
            if let Some(i) = self.order.iter().position(|&s| s == slot) {
                self.index = i;
            }
        }
        if self.index >= self.order.len() {
            self.index = 0;
        }
    }

    fn stop(&mut self, host: &dyn ClientHost) {
        self.floating.clear();
        self.placed = None;
        if let Some(p) = self.now.take() {
            if let Some(s) = p.sound {
                s.stop(host);
            }
        }
        for (s, _) in self.old.drain(..) {
            s.stop(host);
        }
    }

    /// Lets go of the song playing: fading out over `ticks` when smooth changes are on.
    fn release(&mut self, host: &dyn ClientHost, ticks: u32) {
        if !host.setting_bool("crossfade").unwrap_or(true) || ticks == 0 {
            if let Some(p) = self.now.take() {
                if let Some(s) = p.sound {
                    s.stop(host);
                }
            }
        } else if let Some(p) = self.now.take() {
            if let Some(s) = p.sound {
                if p.start.is_some() && p.pause_at.is_none() {
                    s.fade(host, 0.0, ticks);
                    self.old.push((s, host.millis()));
                } else {
                    s.stop(host);
                }
            }
        }
        self.floating.clear();
        self.placed = None;
    }

    /// The live delay of the song playing: the global setting plus this song's own.
    fn offset_of(&self, host: &dyn ClientHost, slot: u32) -> i64 {
        host.setting_f64("lyrics_offset").unwrap_or(0.0) as i64 + self.song_offset.get(&slot).copied().unwrap_or(0)
    }

    /// The , and . keys: the lyrics of this song sooner (+) or later (-).
    fn nudge_song(&mut self, by: i64) {
        let Some(slot) = self.now.as_ref().map(|p| p.song.slot) else {
            self.toast("Pon una canción para ajustar su letra");
            return;
        };
        let v = self.song_offset.entry(slot).or_insert(0);
        *v = (*v + by).clamp(-10_000, 10_000);
        let v = *v;
        let what = if v == 0 { "exacta".to_string() } else if v > 0 { format!("{:.2} s antes", v as f64 / 1000.0) } else { format!("{:.2} s después", -v as f64 / 1000.0) };
        self.toast(&format!("Letra de esta canción: {what}  (guárdalo en PREPARAR: {v:+} ms)"));
    }

    fn toast(&mut self, text: &str) {
        self.toast = Some((text.to_string(), 60));
    }

    fn volume(host: &dyn ClientHost) -> f32 {
        ((host.setting_f64("volume").unwrap_or(100.0) / 100.0) as f32).max(0.0)
    }

    fn play_index(&mut self, host: &dyn ClientHost, all: &[Song]) {
        self.release(host, FADE_OUT_TICKS);
        let Some(&slot) = self.order.get(self.index) else {
            self.toast(NO_SONGS);
            return;
        };
        let Some(song) = all.iter().find(|s| s.slot == slot).cloned() else { return };
        // At the listener, on the jukebox mixer: the game's Music slider doesn't touch it
        // (it plays with Music at 0), and the sound engine pauses it with the game when
        // Escape opens the pause menu, then carries on from the same spot.
        let mut play = SoundPlay::ui(&format!("{ID}:cancion.{slot}"));
        play.source = SoundSource::Records;
        let smooth = host.setting_bool("crossfade").unwrap_or(true) && !self.old.is_empty();
        play.volume = if smooth { (Self::volume(host) * 0.05).max(0.01) } else { Self::volume(host) };
        let sound = host.play_local_sound(&play, 0);
        self.toast = None;
        let no_lyrics = song.lyrics.is_none();
        let style = host.setting_bool("style_random").unwrap_or(false).then(|| {
            let r = ((host.millis() as f32 * 0.001 + song.slot as f32) * 12.9898).sin() * 43758.547;
            ((r - r.floor()) * STYLE_KEYS.len() as f32) as usize % STYLE_KEYS.len()
        });
        let instrumental = song.instrumental();
        let check = song.info.as_ref().is_some_and(|i| i.to_check() && song.lyrics.is_some());
        self.now = Some(Playing { song, sound, style, fade_in: smooth, ..Default::default() });
        if no_lyrics && !instrumental && host.setting_bool("lyrics").unwrap_or(true) {
            self.toast("Esta canción no tiene letra: búscala en PREPARAR_CANCIONES.bat");
        }
        let _ = check;
    }

    fn step(&mut self, host: &dyn ClientHost, by: i64) {
        let all = songs();
        let shuffle = host.setting_bool("shuffle").unwrap_or(false);
        if self.order.len() != all.len() {
            self.rebuild_order(&all, shuffle);
        }
        if self.order.is_empty() {
            self.stop(host);
            self.toast(NO_SONGS);
            return;
        }
        let n = self.order.len() as i64;
        self.index = (((self.index as i64 + by) % n + n) % n) as usize;
        self.play_index(host, &all);
    }

    /// "Previous": back to the start of the song first, the one before when just started.
    fn previous(&mut self, host: &dyn ClientHost) {
        let now = host.millis();
        let restart = self.now.as_ref().is_some_and(|p| p.ms(now) > 3000);
        self.step(host, if restart { 0 } else { -1 });
    }

    /// Plays the song in `slot`, chosen from the menu.
    fn play_slot(&mut self, host: &dyn ClientHost, slot: u32) {
        let all = songs();
        let shuffle = host.setting_bool("shuffle").unwrap_or(false);
        self.rebuild_order(&all, shuffle);
        if let Some(i) = self.order.iter().position(|&s| s == slot) {
            self.index = i;
            self.play_index(host, &all);
        }
    }

    /// The volume setting changed: the song playing follows it at once.
    fn apply_volume(&self, host: &dyn ClientHost) {
        if let Some(p) = &self.now {
            if let (Some(sound), false) = (p.sound, p.quiet) {
                sound.fade(host, Self::volume(host), 2);
            }
        }
    }

    fn toggle(&mut self, host: &dyn ClientHost) {
        if self.now.is_some() {
            self.release(host, FADE_OUT_TICKS);
            self.toast("Música detenida");
        } else {
            let all = songs();
            let shuffle = host.setting_bool("shuffle").unwrap_or(false);
            self.rebuild_order(&all, shuffle);
            self.play_index(host, &all);
        }
    }

    /// The lyric clock starts on the frame the sound is first heard (not on the next tick).
    fn follow_start(&mut self, host: &dyn ClientHost) {
        let volume = Self::volume(host);
        let Some(p) = &mut self.now else { return };
        if p.start.is_none() && matches!(p.sound.and_then(|s| s.state(host)), Some(SoundState::Playing)) {
            p.start = Some(host.millis());
            if p.fade_in {
                p.fade_in = false;
                if let Some(s) = p.sound {
                    s.fade(host, volume, FADE_IN_TICKS);
                }
            }
        }
    }

    /// Follows the game's pause so the lyric clock stops exactly while the sound does.
    fn follow_pause(&mut self, host: &dyn ClientHost) {
        let now = host.millis();
        let paused = host.is_paused();
        let Some(p) = &mut self.now else { return };
        if p.start.is_none() {
            return;
        }
        match (paused, p.pause_at) {
            (true, None) => p.pause_at = Some(now),
            (false, Some(at)) => {
                p.paused += now.saturating_sub(at);
                p.pause_at = None;
            }
            _ => {}
        }
    }

    /// Called each client tick: follow the sound and move on when it ends.
    fn tick(&mut self, host: &dyn ClientHost) {
        self.tick += 1;
        if let Some((_, left)) = &mut self.toast {
            *left = left.saturating_sub(1);
            if *left == 0 {
                self.toast = None;
            }
        }
        self.follow_start(host);
        self.follow_pause(host);
        let now = host.millis();
        // Songs fading out: forget them once they're silent (or after a few seconds).
        let mut keep = Vec::new();
        for (s, at) in self.old.drain(..) {
            let over = !matches!(s.state(host), Some(SoundState::Playing) | Some(SoundState::Waiting));
            if over || now.saturating_sub(at) > 4000 {
                if !over {
                    s.stop(host);
                }
            } else {
                keep.push((s, at));
            }
        }
        self.old = keep;
        // Multiplayer doesn't pause: the pause menu turns the song down instead.
        let menu_up = now.saturating_sub(self.pause_menu_seen) < 300 && !host.is_paused();
        let volume = Self::volume(host);
        let Some(p) = &mut self.now else { return };
        if let Some(sound) = p.sound {
            if menu_up && !p.quiet {
                p.quiet = true;
                sound.fade(host, 0.02, 4);
            } else if !menu_up && p.quiet {
                p.quiet = false;
                sound.fade(host, volume, 4);
            }
        }
        let state = p.sound.and_then(|s| s.state(host));
        match state {
            Some(SoundState::Waiting) | None if p.start.is_none() => {
                // Still loading; give up on a sound that never starts (no file).
                if !host.is_paused() {
                    p.ticks += 1;
                }
                if p.ticks > 100 {
                    let slot = p.song.slot;
                    self.now = None;
                    self.toast(&format!("No se pudo reproducir la canción {slot}"));
                }
            }
            Some(SoundState::Playing) | Some(SoundState::Waiting) => {
                if p.start.is_none() {
                    p.start = Some(now);
                }
                // Smooth change: the next song comes in over the last seconds of this one.
                let left = p.song.duration.map(|d| d - p.ms(now));
                if host.setting_bool("crossfade").unwrap_or(true)
                    && !p.ending
                    && p.pause_at.is_none()
                    && !host.is_paused()
                    && p.song.duration.is_some_and(|d| d > 20_000)
                    && left.is_some_and(|l| l <= CROSSFADE_MS)
                {
                    p.ending = true;
                    let repeat = host.setting_choice("repeat").unwrap_or(0);
                    let last = self.index + 1 >= self.order.len();
                    self.release(host, 30);
                    match repeat {
                        1 => self.step(host, 0),
                        2 if last => {}
                        _ => self.step(host, 1),
                    }
                }
            }
            _ => {
                // It ended on its own: next one, as "repeat" says.
                let repeat = host.setting_choice("repeat").unwrap_or(0);
                let last = self.index + 1 >= self.order.len();
                self.now = None;
                self.floating.clear();
                self.placed = None;
                match repeat {
                    1 => self.step(host, 0),
                    2 if last => {}
                    _ => self.step(host, 1),
                }
            }
        }
    }
}

// ---------------------------------------------------------------- drawing

/// A line of text with its left edge at `x` and top at `y`, scaled by `scale`.
fn paint(draw: &mut dyn DrawContext, text: &str, x: f32, y: f32, scale: f32, color: u32, shadow: bool) {
    if BOLD.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = draw.gui_paint(&[
            GuiDraw::Push(vec![mod_api::client::hud_layout::PoseOp::Translate(x + scale * 0.6, y), mod_api::client::hud_layout::PoseOp::Scale(scale, scale)]),
            GuiDraw::Text { text: lit(text, ttf()), x: 0, y: 0, color, shadow: false },
            GuiDraw::Pop,
        ]);
    }
    let _ = draw.gui_paint(&[
        GuiDraw::Push(vec![mod_api::client::hud_layout::PoseOp::Translate(x, y), mod_api::client::hud_layout::PoseOp::Scale(scale, scale)]),
        GuiDraw::Text { text: lit(text, ttf()), x: 0, y: 0, color, shadow },
        GuiDraw::Pop,
    ]);
}

/// A line of text at `(x, y)` (its centre when `centered`), scaled by `scale`.
fn scaled_text(draw: &mut dyn DrawContext, text: &str, x: f32, y: f32, scale: f32, color: u32, centered: bool) {
    let w = tw(draw, text) * scale;
    let left = if centered { x - w / 2.0 } else { x };
    paint(draw, text, left, y, scale, color, true);
}

/// A song's rhythm file, read: loudness and bass per 50 ms, and its tempo through the song.
#[derive(Clone, Debug, Default)]
pub struct Rhythm {
    pub level: Vec<u8>,
    pub bass: Vec<u8>,
    /// Tempo found in windows of the song: (from frame, beat length in frames, first beat frame).
    pub tempo: Vec<(usize, f32, f32)>,
}

impl Rhythm {
    /// Line one is the loudness, line two (newer files) the bass, one digit per 50 ms.
    pub fn parse(text: &[u8]) -> Self {
        let mut lines = text.split(|c| *c == b'\n').map(parse_energy).filter(|l| !l.is_empty());
        let level = lines.next().unwrap_or_default();
        let bass = lines.next().unwrap_or_default();
        let mut r = Rhythm { level, bass, tempo: Vec::new() };
        r.tempo = find_tempo(r.drive());
        r
    }

    /// What beats are found in: the bass when there is one.
    fn drive(&self) -> &[u8] {
        if self.bass.len() * 2 >= self.level.len() && !self.bass.is_empty() { &self.bass } else { &self.level }
    }

    pub fn beat(&self, ms: i64) -> Beat {
        let mut b = beat_at(self.drive(), ms);
        b.level = beat_at(&self.level, ms).level;
        if ms >= 0 {
            let f = ms as f32 / 50.0;
            let i = f as usize;
            if let Some(&(_, period, first)) = self.tempo.iter().rev().find(|t| t.0 <= i) {
                let x = (f - first) / period;
                b.phase = x - x.floor();
                b.count = x.floor();
                b.bpm = 1200.0 / period;
                b.tempo = (-(b.phase * period * 50.0) / 120.0).exp();
            }
        }
        b
    }
}

/// The beat in a stretch of loudness: the beat length (frames) whose repeats line up
/// best with the jumps, then where its beats fall.
fn tempo_of(o: &[f32]) -> Option<(f32, f32)> {
    let at = |t: f32| -> f32 {
        let i = t as usize;
        let f = t - i as f32;
        o.get(i).copied().unwrap_or(0.0) * (1.0 - f) + o.get(i + 1).copied().unwrap_or(0.0) * f
    };
    let total: f32 = o.iter().sum();
    if total < 0.5 || o.len() < 60 {
        return None;
    }
    let mut best = (0.0f32, 10.0f32);
    let mut lag = 6.0f32;
    while lag <= 24.0 {
        let mut s = 0.0f32;
        let mut n = 0.0f32;
        let mut t = 0usize;
        while (t as f32 + lag) < o.len() as f32 - 1.0 {
            s += o[t] * at(t as f32 + lag);
            n += 1.0;
            t += 1;
        }
        // Tempos near 120 a minute are likelier.
        let pref = (-((lag / 10.0).ln()).powi(2) / 0.5).exp();
        let score = s / n.max(1.0) * (0.6 + 0.4 * pref);
        if score > best.0 {
            best = (score, lag);
        }
        lag += 0.05;
    }
    let period = best.1;
    let mut phase = (0.0f32, 0.0f32);
    let mut p = 0.0f32;
    while p < period {
        let mut s = 0.0;
        let mut t = p;
        while t < o.len() as f32 {
            s += at(t);
            t += period;
        }
        if s > phase.0 {
            phase = (s, p);
        }
        p += 0.25;
    }
    Some((period, phase.1))
}

/// The tempo every 10 s, each from the 20 s around it, so a mashup's tempo changes follow.
pub fn find_tempo(e: &[u8]) -> Vec<(usize, f32, f32)> {
    let o: Vec<f32> = (0..e.len())
        .map(|i| if i == 0 { 0.0 } else { (e[i] as f32 - e[i - 1] as f32).max(0.0) / 9.0 })
        .collect();
    let mut out = Vec::new();
    let mut from = 0usize;
    while from < o.len() {
        let end = (from + 400).min(o.len());
        if let Some((period, first)) = tempo_of(&o[from..end]) {
            out.push((from, period, from as f32 + first));
        }
        if end == o.len() {
            break;
        }
        from += 200;
    }
    out
}

/// Fonts: the line's letters swapped for other Unicode letters (the game draws them).
pub const FONT_NAMES: [&str; 40] = [
    "Normal", "Gruesa", "MAYÚSCULAS", "Versalitas", "Ancha", "Círculos", "Cuadros", "Negrita", "Cursiva",
    "Caligrafía", "Gótica", "Doble", "Máquina", "Al revés", "Espaciada", "Burbujas negras", "Cuadros negros",
    "Sans negrita", "Cursiva negrita", "Diminuta", "Tachada", "Subrayada", "Corazones", "Estrellas", "minúsculas",
    "Sans", "Cursiva clásica", "Cursiva clásica negrita", "Gótica fina", "Paréntesis", "Ondulada", "Barrada",
    "Doble subrayado", "Sobrelineada", "Griega", "Rusa", "Corchetes", "Notas", "Espejo", "AlTeRnAdA",
];
const FONT_KEYS: [&str; 40] = [
    "normal", "bold", "upper", "smallcaps", "wide", "circles", "squares", "serif", "italic", "script",
    "gothic", "double", "mono", "upside", "spaced", "black_circles", "black_squares", "sans_bold",
    "bold_italic", "tiny", "strike", "underline", "hearts", "stars", "lower", "sans", "serif_italic",
    "serif_bold_italic", "fraktur", "paren", "wavy", "slash", "double_under", "overline", "greek",
    "cyrillic", "brackets", "notes", "mirror", "alternate",
];

fn plain_letter(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'Á' | 'À' | 'Ä' | 'Â' => 'A',
        'É' | 'È' | 'Ë' | 'Ê' => 'E',
        'Í' | 'Ì' | 'Ï' | 'Î' => 'I',
        'Ó' | 'Ò' | 'Ö' | 'Ô' => 'O',
        'Ú' | 'Ù' | 'Ü' | 'Û' => 'U',
        'Ñ' => 'N',
        c => c,
    }
}

fn math(c: char, upper: u32, lower: u32, digit: Option<u32>, holes: &[(char, char)]) -> char {
    if let Some(&(_, h)) = holes.iter().find(|(k, _)| *k == c) {
        return h;
    }
    let off = match c {
        'A'..='Z' => upper + (c as u32 - 'A' as u32),
        'a'..='z' => lower + (c as u32 - 'a' as u32),
        '0'..='9' => match digit {
            Some(d) => d + (c as u32 - '0' as u32),
            None => return c,
        },
        _ => return c,
    };
    char::from_u32(off).unwrap_or(c)
}

/// The letters of `text` in font `font`.
pub fn styled(text: &str, font: usize) -> String {
    const SMALL: &str = "ᴀʙᴄᴅᴇꜰɢʜɪᴊᴋʟᴍɴᴏᴘǫʀꜱᴛᴜᴠᴡxʏᴢ";
    const FLIP_FROM: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,!?¡¿'\"()[]{}<>_&;";
    const FLIP_TO: &str = "ɐqɔpǝɟƃɥᴉɾʞlɯuodbɹsʇnʌʍxʎz∀ꓭƆꓷƎℲ⅁HIſꓘ⅂WNOԀΌꓤS⊥∩ΛMX⅄Z0ƖᄅƐㄣϛ9ㄥ86˙'¡¿!¿,„)(][}{><‾⅋؛";
    let double = [('C', 'ℂ'), ('H', 'ℍ'), ('N', 'ℕ'), ('P', 'ℙ'), ('Q', 'ℚ'), ('R', 'ℝ'), ('Z', 'ℤ')];
    let each = |f: &dyn Fn(char) -> char| text.chars().map(|c| f(plain_letter(c))).collect::<String>();
    match font {
        2 => text.to_uppercase(),
        3 => text
            .chars()
            .map(|c| {
                let p = plain_letter(c).to_ascii_lowercase();
                if p.is_ascii_lowercase() && c.is_lowercase() {
                    SMALL.chars().nth((p as u8 - b'a') as usize).unwrap_or(c)
                } else {
                    c
                }
            })
            .collect(),
        4 => each(&|c| if ('!'..='~').contains(&c) { char::from_u32(c as u32 - 0x21 + 0xFF01).unwrap_or(c) } else if c == ' ' { '\u{3000}' } else { c }),
        5 => each(&|c| match c {
            'A'..='Z' => char::from_u32(0x24B6 + c as u32 - 'A' as u32).unwrap_or(c),
            'a'..='z' => char::from_u32(0x24D0 + c as u32 - 'a' as u32).unwrap_or(c),
            '0' => '⓪',
            '1'..='9' => char::from_u32(0x2460 + c as u32 - '1' as u32).unwrap_or(c),
            c => c,
        }),
        6 => each(&|c| match c.to_ascii_uppercase() {
            u @ 'A'..='Z' => char::from_u32(0x1F130 + u as u32 - 'A' as u32).unwrap_or(c),
            _ => c,
        }),
        7 => each(&|c| math(c, 0x1D400, 0x1D41A, Some(0x1D7CE), &[])),
        8 => each(&|c| math(c, 0x1D608, 0x1D622, Some(0x1D7E2), &[])),
        9 => each(&|c| math(c, 0x1D4D0, 0x1D4EA, Some(0x1D7CE), &[])),
        10 => each(&|c| math(c, 0x1D56C, 0x1D586, None, &[])),
        11 => each(&|c| math(c, 0x1D538, 0x1D552, Some(0x1D7D8), &double)),
        12 => each(&|c| math(c, 0x1D670, 0x1D68A, Some(0x1D7F6), &[])),
        13 => text
            .chars()
            .rev()
            .map(|c| {
                let c = plain_letter(c);
                FLIP_FROM.chars().position(|f| f == c).and_then(|i| FLIP_TO.chars().nth(i)).unwrap_or(c)
            })
            .collect(),
        15 => each(&|c| match c.to_ascii_uppercase() {
            u @ 'A'..='Z' => char::from_u32(0x1F150 + u as u32 - 'A' as u32).unwrap_or(c),
            _ => c,
        }),
        16 => each(&|c| match c.to_ascii_uppercase() {
            u @ 'A'..='Z' => char::from_u32(0x1F170 + u as u32 - 'A' as u32).unwrap_or(c),
            _ => c,
        }),
        17 => each(&|c| math(c, 0x1D5D4, 0x1D5EE, Some(0x1D7EC), &[])),
        18 => each(&|c| math(c, 0x1D63C, 0x1D656, None, &[])),
        19 => {
            const TINY: &str = "ᵃᵇᶜᵈᵉᶠᵍʰⁱʲᵏˡᵐⁿᵒᵖqʳˢᵗᵘᵛʷˣʸᶻ";
            const DIG: &str = "⁰¹²³⁴⁵⁶⁷⁸⁹";
            each(&|c| {
                let l = c.to_ascii_lowercase();
                if l.is_ascii_lowercase() {
                    TINY.chars().nth((l as u8 - b'a') as usize).unwrap_or(c)
                } else if c.is_ascii_digit() {
                    DIG.chars().nth((c as u8 - b'0') as usize).unwrap_or(c)
                } else {
                    c
                }
            })
        }
        20 | 21 => {
            let mark = if font == 20 { '\u{0336}' } else { '\u{0332}' };
            let mut s = String::new();
            for c in text.chars() {
                s.push(c);
                if !c.is_whitespace() {
                    s.push(mark);
                }
            }
            s
        }
        25 => each(&|c| math(c, 0x1D5A0, 0x1D5BA, Some(0x1D7E2), &[])),
        26 => each(&|c| math(c, 0x1D434, 0x1D44E, None, &[('h', 'ℎ')])),
        27 => each(&|c| math(c, 0x1D468, 0x1D482, None, &[])),
        28 => each(&|c| math(c, 0x1D504, 0x1D51E, None, &[('C', 'ℭ'), ('H', 'ℌ'), ('I', 'ℑ'), ('R', 'ℜ'), ('Z', 'ℨ')])),
        29 => each(&|c| {
            let l = c.to_ascii_lowercase();
            if l.is_ascii_lowercase() {
                char::from_u32(0x249C + (l as u32 - 'a' as u32)).unwrap_or(c)
            } else if ('1'..='9').contains(&c) {
                char::from_u32(0x2474 + (c as u32 - '1' as u32)).unwrap_or(c)
            } else {
                c
            }
        }),
        30..=33 => {
            let mark = ['\u{0334}', '\u{0338}', '\u{0333}', '\u{0305}'][font - 30];
            let mut s = String::new();
            for c in text.chars() {
                s.push(c);
                if !c.is_whitespace() {
                    s.push(mark);
                }
            }
            s
        }
        34 | 35 => {
            const GREEK: &str = "αβςδεϝγηιϳκλμνορϙρστυνωχψζ";
            const CYR: &str = "абсдэфгніјклмиорҩяѕтцвшжуз";
            let table = if font == 34 { GREEK } else { CYR };
            each(&|c| {
                let l = c.to_ascii_lowercase();
                if l.is_ascii_lowercase() { table.chars().nth((l as u8 - b'a') as usize).unwrap_or(c) } else { c }
            })
        }
        36 => text.split_whitespace().map(|w| format!("【{w}】")).collect::<Vec<_>>().join(""),
        37 => text.split_whitespace().collect::<Vec<_>>().join(" ♪ "),
        38 => text.chars().rev().collect(),
        39 => {
            let mut up = false;
            text.chars()
                .map(|c| {
                    if c.is_alphabetic() {
                        up = !up;
                        if up { c.to_uppercase().next().unwrap_or(c) } else { c.to_lowercase().next().unwrap_or(c) }
                    } else {
                        c
                    }
                })
                .collect()
        }
        22 => text.split_whitespace().map(|w| format!("{w}♥")).collect::<Vec<_>>().join(" "),
        23 => text.split_whitespace().map(|w| format!("✦{w}")).collect::<Vec<_>>().join(" "),
        24 => text.to_lowercase(),
        14 => {
            let mut s = String::new();
            for (i, c) in text.chars().enumerate() {
                if i > 0 && c != ' ' {
                    s.push(' ');
                }
                s.push(if c == ' ' { ' ' } else { c });
            }
            s
        }
        _ => text.to_string(),
    }
}

/// Every lyric line in font `font`.
pub fn styled_lyrics(l: &Lyrics, font: usize) -> Lyrics {
    if font == 0 || font == 1 {
        return l.clone();
    }
    let mut out = l.clone();
    for line in &mut out.lines {
        line.text = styled(&line.text, font);
        // Word times stay only when the words are still the same ones, in order.
        if font == 13 || line.text.split_whitespace().count() != line.words.len() {
            line.words.clear();
        }
    }
    out
}

/// Real fonts shipped with the mod (assets/musica/font): their file names; 0 is Minecraft's own.
pub const TTF_KEYS: [&str; 17] = [
    "minecraft", "poppins", "bebas", "bangers", "press_start", "silkscreen", "orbitron", "audiowide", "lobster",
    "pacifico", "righteous", "marker", "luckiest", "creepster", "cinzel", "special_elite", "comfortaa",
];
pub const TTF_NAMES: [&str; 17] = [
    "Minecraft", "Poppins", "Bebas Neue", "Bangers", "Press Start", "Silkscreen", "Orbitron", "Audiowide", "Lobster",
    "Pacifico", "Righteous", "Permanent Marker", "Luckiest Guy", "Creepster", "Cinzel", "Special Elite", "Comfortaa",
];
/// The real font the lyrics draw with now (TTF_KEYS); 0 outside the lyrics.
static TTF: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub fn set_ttf(i: usize) {
    TTF.store(i.min(TTF_KEYS.len() - 1), std::sync::atomic::Ordering::Relaxed);
}
fn ttf() -> usize {
    TTF.load(std::sync::atomic::Ordering::Relaxed)
}
/// The chosen real font (setting "lyrics_ttf").
pub fn ttf_setting(host: &dyn ClientHost) -> usize {
    (host.setting_choice("lyrics_ttf").unwrap_or(0) as usize).min(TTF_KEYS.len() - 1)
}
/// Each letter's width in each real font, measured by the game.
static TTF_WIDTHS: std::sync::Mutex<Option<std::collections::HashMap<(usize, char), f32>>> = std::sync::Mutex::new(None);
/// A text component in the real font `f`.
fn lit(text: &str, f: usize) -> Text {
    let mut t = Text::literal(text);
    if f > 0 {
        t.style.font = Some(format!("musica:{}", TTF_KEYS[f]));
    }
    t
}
/// Measures the letters of `text` in font `f` (once each), so lines can be laid out.
pub fn prepare_ttf(host: &dyn ClientHost, f: usize, text: &str) {
    if f == 0 {
        return;
    }
    let mut guard = TTF_WIDTHS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(Default::default);
    for c in text.chars() {
        if c.is_whitespace() && c != ' ' {
            continue;
        }
        if !map.contains_key(&(f, c)) {
            if let Ok(w) = host.text_width(&lit(&c.to_string(), f)) {
                map.insert((f, c), w as f32);
            }
        }
    }
}
/// The width of `text` as the lyrics draw it now (in the real font when one is chosen).
fn tw(draw: &dyn DrawContext, text: &str) -> f32 {
    let f = ttf();
    if f == 0 {
        return draw.text_width(text);
    }
    let guard = TTF_WIDTHS.lock().unwrap_or_else(|e| e.into_inner());
    text.chars()
        .map(|c| guard.as_ref().and_then(|m| m.get(&(f, c)).copied()).unwrap_or_else(|| draw.text_width(&c.to_string())))
        .sum()
}

/// Gruesa: every letter drawn twice, a hair apart.
static BOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// How opaque the lyrics are (setting "lyrics_opacity"), as f32 bits.
static LYRIC_ALPHA: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);
pub fn set_lyric_alpha(a: f32) {
    LYRIC_ALPHA.store(a.clamp(0.05, 1.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
}
fn lyric_alpha() -> f32 {
    f32::from_bits(LYRIC_ALPHA.load(std::sync::atomic::Ordering::Relaxed))
}
pub fn set_bold(on: bool) {
    BOLD.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// How a lyric line is drawn: the style, the rhythm and its settings.
#[derive(Clone, Copy, Default)]
pub struct Look {
    pub style: usize,
    /// 0..1: how hard words pulse.
    pub rhythm: f32,
    /// What the pulse does (FX_NAMES).
    pub fx: usize,
    /// What drives the pulse: auto, the words, the music.
    pub source: usize,
    pub boxed: bool,
    /// The colour choice (the last one is the rainbow).
    pub color: usize,
    /// Seconds on the game's clock, for animations.
    pub t: f32,
    /// The music's beat now (0..1, a kick that decays) and loudness.
    pub kick: f32,
    pub level: f32,
    /// The song has a rhythm file.
    pub music: bool,
    /// The tempo: a pulse on every beat, where in the beat we are (0..1), beats so far, tempo.
    pub tempo: f32,
    pub phase: f32,
    pub count: f32,
    pub bpm: f32,
}

const FX_KEYS: [&str; 24] = [
    "jump", "zoom", "glow", "shake", "color", "sway", "wave", "flash", "elastic", "heartbeat", "alternate", "stairs", "strobe",
    "float", "all", "soft_bounce", "pendulum", "breathe", "drop", "mirror", "beat_rainbow", "ripple", "kick_side", "double_beat",
];
pub const FX_NAMES: [&str; 24] = [
    "Salto", "Zoom", "Brillo", "Temblor", "Color", "Balanceo", "Ola", "Destello", "Elástico", "Latido", "Alterno", "Escalera",
    "Parpadeo", "Flotar", "Todo", "Rebote suave", "Péndulo", "Respirar", "Caída", "Espejo", "Arcoíris al ritmo",
    "Onda expansiva", "Golpe lateral", "Latido doble",
];
const FX_ALL: usize = 14;
const SOURCE_KEYS: [&str; 4] = ["auto", "words", "music", "tempo"];
pub const SOURCE_NAMES: [&str; 4] = ["Automático", "Palabras", "Golpes", "Tempo"];

impl Look {
    pub fn from(host: &dyn ClientHost) -> Self {
        Look {
            style: (host.setting_choice("lyrics_style").unwrap_or(1) as usize).min(STYLE_KEYS.len() - 1),
            rhythm: (host.setting_f64("rhythm").unwrap_or(60.0) / 100.0) as f32,
            fx: (host.setting_choice("rhythm_fx").unwrap_or(14) as usize).min(FX_KEYS.len() - 1),
            source: (host.setting_choice("rhythm_source").unwrap_or(0) as usize).min(SOURCE_KEYS.len() - 1),
            boxed: host.setting_bool("lyrics_box").unwrap_or(false),
            color: host.setting_choice("lyrics_color").unwrap_or(0) as usize,
            t: (host.millis() % 3_600_000) as f32 / 1000.0,
            ..Default::default()
        }
    }

    pub fn with_beat(mut self, b: Beat) -> Self {
        self.kick = b.kick;
        self.level = b.level;
        self.tempo = b.tempo;
        self.phase = b.phase;
        self.count = b.count;
        self.bpm = b.bpm;
        self.music = true;
        self
    }

    fn rgb(&self, hue: f32) -> u32 {
        COLORS.get(self.color).copied().unwrap_or_else(|| hsv(self.t / 4.0 + hue))
    }

    /// How hard a word pulses now, from its own kick when sung and the music's beat.
    fn pulse(&self, word: f32) -> f32 {
        let music = if self.music { self.kick } else { 0.0 };
        let tempo = if self.music { self.tempo } else { 0.0 };
        let p = match self.source {
            1 => word,
            2 => music,
            3 if self.bpm > 0.0 => tempo,
            _ if self.music => music.max(tempo * 0.6).max(word * 0.5),
            _ => word,
        };
        p * self.rhythm
    }
}

/// The music's loudness at a moment, from the song's rhythm file.
#[derive(Clone, Copy, Default, Debug)]
pub struct Beat {
    pub level: f32,
    pub kick: f32,
    pub tempo: f32,
    pub phase: f32,
    pub count: f32,
    pub bpm: f32,
}

/// A rhythm file: one digit 0..9 of loudness per 50 ms.
pub fn parse_energy(text: &[u8]) -> Vec<u8> {
    text.iter().filter(|c| c.is_ascii_digit()).map(|c| c - b'0').collect()
}

/// The loudness at `ms` and the kick of a beat: a jump over the last moments' average, fading.
pub fn beat_at(e: &[u8], ms: i64) -> Beat {
    if ms < 0 || e.is_empty() {
        return Beat::default();
    }
    let i = (ms / 50) as usize;
    if i >= e.len() {
        return Beat::default();
    }
    let v = |k: usize| e.get(k).copied().unwrap_or(0) as f32 / 9.0;
    let frac = (ms % 50) as f32 / 50.0;
    let level = v(i) * (1.0 - frac) + v(i + 1) * frac;
    let onset = |k: usize| {
        let n = k.min(8);
        if n == 0 {
            return 0.0;
        }
        let avg = (1..=n).map(|d| v(k - d)).sum::<f32>() / n as f32;
        ((v(k) - avg) * 2.4).clamp(0.0, 1.0)
    };
    let mut kick = 0.0f32;
    for d in 0..6usize {
        if d > i {
            break;
        }
        kick = kick.max(onset(i - d) * (-((d as f32 + frac) * 50.0) / 140.0).exp());
    }
    Beat { level, kick, ..Default::default() }
}

/// What the pulse does to one word.
struct Fx {
    dx: f32,
    dy: f32,
    zoom: f32,
    rgb: u32,
}

fn noise(k: usize, t: f32) -> (f32, f32) {
    let f = |a: f32| {
        let v = a.sin() * 43758.547;
        (v - v.floor()) * 2.0 - 1.0
    };
    let step = (t * 24.0).floor();
    (f(k as f32 * 12.9898 + step * 78.233), f(k as f32 * 39.346 + step * 11.135))
}

fn fx(look: &Look, p: f32, k: usize, scale: f32, rgb: u32) -> Fx {
    let all = look.fx == FX_ALL;
    let mut f = Fx { dx: 0.0, dy: 0.0, zoom: 1.0, rgb };
    if look.fx == 0 || all {
        f.dy = -p * 4.0 * scale;
    }
    if look.fx == 1 || all {
        f.zoom = 1.0 + 0.3 * p;
    }
    if look.fx == 2 || all {
        f.rgb = mix(f.rgb, 0xFFFFFF, p * 0.7);
    }
    if look.fx == 3 || all {
        let (a, b) = noise(k, look.t);
        f.dx = a * p * 2.5 * scale;
        f.dy += b * p * 2.5 * scale;
    }
    if look.fx == 4 || all {
        f.rgb = mix(f.rgb, hsv(look.t * 0.7 + k as f32 * 0.13), p.min(1.0));
    }
    let swing = if look.bpm > 0.0 { look.phase + look.count } else { look.t * 2.0 };
    if look.fx == 5 || all {
        f.dx += (swing * std::f32::consts::PI).sin() * (0.6 + 2.0 * p) * scale * look.rhythm.max(0.2);
    }
    if look.fx == 6 || all {
        f.dy += (look.t * 6.0 + k as f32 * 0.8).sin() * (0.5 + 2.5 * p) * scale * 0.6;
    }
    if look.fx == 7 || all {
        f.rgb = mix(f.rgb, 0xFFFFFF, (p * 1.4).min(1.0));
        f.zoom *= 1.0 + 0.08 * p;
    }
    if look.fx == 8 {
        f.zoom *= 1.0 + 0.45 * p * (p * 9.0).cos();
    }
    let beat = if look.music && look.bpm > 0.0 { look.tempo } else { p };
    if look.fx == 9 || all {
        f.zoom *= 1.0 + 0.18 * beat * look.rhythm;
    }
    if look.fx == 10 {
        let side = if (k + look.count as usize) % 2 == 0 { 1.0 } else { -1.0 };
        f.dy += side * beat.max(p) * 3.5 * scale * look.rhythm.max(0.3);
    }
    if look.fx == 11 {
        f.dy -= (k % 4) as f32 * beat.max(p) * 1.6 * scale * look.rhythm.max(0.3);
    }
    if look.fx == 12 {
        f.rgb = mix(f.rgb, 0x202030, (1.0 - beat) * 0.55 * look.rhythm.max(0.3));
    }
    if look.fx == 13 || all {
        f.dy += (look.t * 1.3 + k as f32 * 1.7).sin() * 1.2 * scale;
        f.dx += (look.t * 0.9 + k as f32 * 2.3).cos() * 0.6 * scale;
    }
    let pi = std::f32::consts::PI;
    let r = look.rhythm.max(0.3);
    let ph = if look.bpm > 0.0 { look.phase } else { (look.t * 2.0).fract() };
    match look.fx {
        15 => f.dy -= (ph * pi).sin().abs() * (1.5 + 3.0 * p) * scale * r,
        16 => f.dx += (swing * pi + k as f32 * 0.9).sin() * (1.0 + 2.5 * p) * scale * 0.7 * r,
        17 => f.zoom *= 1.0 + 0.12 * ((swing * pi * 0.5).sin() * 0.5 + 0.5) * r + 0.1 * p,
        18 => f.dy += beat.max(p) * 4.0 * scale * r,
        19 => f.zoom *= if k % 2 == 0 { 1.0 + 0.25 * beat.max(p) } else { 1.0 - 0.15 * beat.max(p) },
        20 => f.rgb = mix(f.rgb, hsv(look.count * 0.17 + k as f32 * 0.08), 0.85),
        21 => f.dy -= (ph * 2.0 * pi - k as f32 * 0.8).sin() * (0.5 + 3.0 * p) * scale * 0.6 * r,
        22 => f.dx += if (look.count as usize) % 2 == 0 { 1.0 } else { -1.0 } * beat.max(p) * 3.0 * scale * r,
        23 => {
            let hb = if look.bpm > 0.0 { (-((ph * 12.0).powi(2))).exp() + 0.7 * (-(((ph - 0.25) * 12.0).powi(2))).exp() } else { p };
            f.zoom *= 1.0 + 0.22 * hb * r;
        }
        _ => {}
    }
    f
}

/// Text with its left edge at `x` (as at `scale`), zoomed `zoom` around its centre.
#[allow(clippy::too_many_arguments)]
fn paint_zoom(draw: &mut dyn DrawContext, text: &str, x: f32, y: f32, scale: f32, zoom: f32, color: u32, shadow: bool) {
    if (zoom - 1.0).abs() < 0.001 {
        paint(draw, text, x, y, scale, color, shadow);
        return;
    }
    let w = tw(draw, text) * scale;
    let s = scale * zoom;
    paint(draw, text, x - (w * zoom - w) / 2.0, y - (s - scale) * 4.0, s, color, shadow);
}

/// A word letter by letter; `each(i)` gives each letter's lift and colour.
#[allow(clippy::too_many_arguments)]
fn paint_chars(
    draw: &mut dyn DrawContext,
    word: &str,
    x: f32,
    y: f32,
    scale: f32,
    zoom: f32,
    alpha: f32,
    mut each: impl FnMut(usize) -> (f32, u32),
) {
    let w = tw(draw, word) * scale;
    let z = scale * zoom;
    let mut lx = x - (w * zoom - w) / 2.0;
    let zy = y - (z - scale) * 4.0;
    for (ci, ch) in word.chars().enumerate() {
        let s = ch.to_string();
        let cw = tw(draw, &s) * z;
        let (dy, rgb) = each(ci);
        paint(draw, &s, lx, zy + dy, z, argb(rgb, alpha), true);
        lx += cw;
    }
}

/// A word letter by letter; `each(i, c)` gives each letter's shift, colour, alpha and
/// maybe another letter to show in its place.
#[allow(clippy::too_many_arguments)]
fn paint_chars4(
    draw: &mut dyn DrawContext,
    word: &str,
    x: f32,
    y: f32,
    scale: f32,
    zoom: f32,
    mut each: impl FnMut(usize, char) -> (f32, f32, u32, f32, Option<char>),
) {
    let w = tw(draw, word) * scale;
    let z = scale * zoom;
    let mut lx = x - (w * zoom - w) / 2.0;
    let zy = y - (z - scale) * 4.0;
    for (ci, ch) in word.chars().enumerate() {
        let s = ch.to_string();
        let cw = tw(draw, &s) * z;
        let (dx, dy, rgb, a, other) = each(ci, ch);
        if a > 0.01 {
            let shown = other.map(|o| o.to_string()).unwrap_or(s);
            paint(draw, &shown, lx + dx, zy + dy, z, argb(rgb, a), true);
        }
        lx += cw;
    }
}

/// When the words of a line are sung: from the line's time to the next line's, at a
/// singing pace, each word getting time for its letters.
pub fn sung_span(lyrics: &Lyrics, i: usize) -> i64 {
    let line = &lyrics.lines[i];
    let next_at = lyrics.next_at(i);
    if let Some(&last) = line.words.last() {
        let next = next_at.map(|n| n - line.at).unwrap_or(last + 800);
        return (last + 450).min(next.max(last + 150)).max(250);
    }
    let chars = line.text.chars().count().max(1) as i64;
    let gap = next_at.map(|n| n - line.at).unwrap_or(chars * 90 + 1500).max(300);
    (gap * 92 / 100).min((chars * 85).max(800)).max(250)
}

/// Sparks and the screen flash were taken out of the menu (1.11); they stay off.
const NO_EXTRAS: bool = false;

/// Sparks thrown from `(cx, cy)` on every beat of the tempo, fading before the next.
/// They need no memory: each beat's sparks come from its number.
pub fn sparks(draw: &mut dyn DrawContext, look: &Look, cx: f32, cy: f32, spread: f32, scale: f32) {
    if !look.music || look.bpm <= 0.0 || look.rhythm < 0.01 {
        return;
    }
    let age = look.phase;
    let strength = (0.4 + look.level).min(1.0) * look.rhythm.max(0.3);
    let n = 10 + (look.level * 10.0) as usize;
    for i in 0..n {
        let (a, b) = noise(i * 7 + 3, look.count);
        let (c, d) = noise(i * 13 + 5, look.count + 0.5);
        let x0 = cx + a * spread * 0.5;
        let vx = c * 40.0 * scale;
        let vy = -(20.0 + (d + 1.0) * 30.0) * scale;
        let t = age * 1.2;
        let x = x0 + vx * t;
        let y = cy + b * 6.0 * scale + vy * t + 45.0 * scale * t * t;
        let alpha = (1.0 - age).powi(2) * strength;
        if alpha < 0.03 {
            continue;
        }
        let sz = (1.0 + (1.0 - age) * 1.5) * scale.max(1.0) * 0.6;
        let rgb = mix(hsv(look.t * 0.2 + i as f32 * 0.09), 0xFFFFFF, 0.35);
        draw.fill([x - sz / 2.0, y - sz / 2.0, sz, sz], argb(rgb, alpha));
    }
}

/// A coloured glow at the screen's edges on each beat.
pub fn edge_flash(draw: &mut dyn DrawContext, look: &Look, w: f32, h: f32) {
    let b = if look.bpm > 0.0 { look.tempo } else { look.kick };
    let a = b * 0.45 * look.rhythm.max(0.3);
    if a < 0.02 {
        return;
    }
    let rgb = hsv(look.t * 0.15);
    let e = 28.0;
    draw.fill_gradient([0.0, 0.0, w, e], argb(rgb, a), argb(rgb, 0.0));
    draw.fill_gradient([0.0, h - e, w, e], argb(rgb, 0.0), argb(rgb, a));
}

/// Draws one lyric line centred at `(cx, cy)`, `scale` big, wrapped at `max` GUI pixels.
/// `elapsed` is ms since the line began (None draws it fully sung), `sung` how long its words take.
#[allow(clippy::too_many_arguments)]
pub fn render_line(
    draw: &mut dyn DrawContext,
    text: &str,
    cx: f32,
    cy: f32,
    scale: f32,
    alpha: f32,
    look: &Look,
    hue: f32,
    elapsed: Option<i64>,
    sung: i64,
    max: f32,
    timing: &[i64],
) {
    let alpha = alpha * lyric_alpha();
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || alpha <= 0.01 {
        return;
    }
    let total: usize = words.iter().map(|w| w.chars().count() + 1).sum::<usize>().max(1);
    let space = tw(draw, " ");
    let widths: Vec<f32> = words.iter().map(|w| tw(draw, w)).collect();
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut row_w = 0.0;
    for (k, w) in widths.iter().enumerate() {
        let empty = rows.last().is_none_or(|r| r.is_empty());
        let add = if empty { *w } else { space + w };
        if !empty && (row_w + add) * scale > max {
            rows.push(vec![k]);
            row_w = *w;
        } else if let Some(r) = rows.last_mut() {
            r.push(k);
            row_w += add;
        }
    }
    let row_h = 11.0 * scale;
    let block_h = rows.len() as f32 * row_h;
    let top = cy - block_h / 2.0;

    let mut onsets = Vec::with_capacity(words.len());
    let mut starts = Vec::with_capacity(words.len());
    let mut acc = 0usize;
    for w in &words {
        onsets.push(sung * acc as i64 / total as i64);
        starts.push(acc);
        acc += w.chars().count() + 1;
    }
    let mut lens: Vec<i64> = words.iter().map(|w| (sung * (w.chars().count() + 1) as i64 / total as i64).max(1)).collect();
    // Real word times, when the lyrics have them: each word lights exactly when it's sung.
    if timing.len() == words.len() {
        for k in 0..words.len() {
            onsets[k] = timing[k].max(0);
            let end = timing.get(k + 1).copied().unwrap_or(sung.max(timing[k] + 300));
            lens[k] = (end - timing[k]).clamp(60, 1500);
        }
    }
    let el = elapsed.unwrap_or(i64::MAX / 4);
    let pop_of = |k: usize| {
        let tau = el - onsets[k];
        if tau < 0 || elapsed.is_none() { 0.0 } else { (-(tau as f32) / 170.0).exp() }
    };
    let beat = (0..words.len()).map(pop_of).fold(0.0f32, f32::max);
    let line_alpha = match (look.style, elapsed) {
        (0 | 5 | 11, Some(e)) => alpha * (e as f32 / 250.0).clamp(0.0, 1.0),
        _ => alpha,
    };
    let line_kick = if look.style == 0 { 1.0 + 0.08 * look.pulse(beat) } else { 1.0 };
    let typed = if elapsed.is_none() { usize::MAX } else { (el.max(0) as f64 * total as f64 / sung.max(1) as f64) as usize };
    let blink = (look.t * 2.5).fract() < 0.5;
    let singing = (0..words.len()).rev().find(|&k| el >= onsets[k]);

    if look.boxed || look.style == 5 {
        let widest = rows
            .iter()
            .map(|row| row.iter().map(|&k| widths[k]).sum::<f32>() + space * row.len().saturating_sub(1) as f32)
            .fold(0.0f32, f32::max)
            * scale;
        let pad = 4.0 * scale;
        draw.fill(
            [cx - widest / 2.0 - pad, top - pad * 0.6, widest + pad * 2.0, block_h + pad * 0.8],
            argb(0x000000, 0.55 * line_alpha),
        );
        if look.style == 5 {
            let f = (el.min(sung) as f32 / sung.max(1) as f32).clamp(0.0, 1.0);
            draw.fill(
                [cx - widest / 2.0 - pad, top + block_h + pad * 0.2 - scale, (widest + pad * 2.0) * f, scale.max(1.0)],
                argb(look.rgb(hue), 0.8 * line_alpha),
            );
        }
    }

    // How much each word grows this frame: its neighbours make room, so words never overlap.
    let grow_of = |k: usize| {
        let tau = el - onsets[k];
        let mut p = look.pulse(pop_of(k));
        if tau < 0 {
            p *= 0.4;
        }
        let mut z = fx(look, p, k, scale, 0).zoom;
        if look.style == 2 && tau >= 0 {
            z *= 1.0 + (0.55 * look.rhythm + 0.15) * (-(tau as f32) / 110.0).exp();
        }
        z.max(1.0)
    };
    for (ri, row) in rows.iter().enumerate() {
        let zooms: Vec<f32> = row.iter().map(|&k| grow_of(k)).collect();
        let w: f32 = row.iter().zip(&zooms).map(|(&k, z)| widths[k] * z).sum::<f32>() + space * row.len().saturating_sub(1) as f32;
        let mut x = cx - w * scale * line_kick / 2.0;
        let y = top + ri as f32 * row_h;
        for (j, &k) in row.iter().enumerate() {
            let word = words[k];
            let s = scale * line_kick;
            let ww = widths[k] * s;
            let slot = ww * zooms[j];
            let tau = el - onsets[k];
            let done = ((tau as f32) / lens[k] as f32).clamp(0.0, 1.0);
            let mut p = look.pulse(pop_of(k));
            if tau < 0 {
                p *= 0.4;
            }
            let f = fx(look, p, k, scale, look.rgb(hue + k as f32 * 0.05));
            let nudge = space * s * 0.45;
            let (wx, wy) = (x + (slot - ww) / 2.0 + f.dx.clamp(-nudge, nudge), y + f.dy);
            let lit = if tau >= 0 { 1.0 } else { 0.45 };
            match look.style {
                // Karaoke: the line waits dim, each word fills with colour as it is sung.
                1 => {
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(0x8A8A8A, line_alpha * 0.9), true);
                    if done > 0.0 {
                        let zw = ww * f.zoom;
                        let zx = wx - (zw - ww) / 2.0;
                        let clip = draw.push_clip([zx - 1.0, wy - row_h, zw * done + 1.0, row_h * 3.0]).is_ok();
                        paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha), true);
                        if clip {
                            let _ = draw.pop_clip();
                        }
                    }
                }
                // Bounce: words pop in one by one, jumping as they're sung.
                2 => {
                    if tau >= 0 {
                        let t = tau as f32;
                        let grow = 1.0 + (0.55 * look.rhythm + 0.15) * (-t / 110.0).exp();
                        let hop = (t / 95.0).sin().abs() * (-t / 260.0).exp() * 5.0 * scale * (0.3 + look.rhythm);
                        let a = (t / 90.0).clamp(0.0, 1.0) * line_alpha;
                        paint_zoom(draw, word, wx, wy - hop, s, grow * f.zoom, argb(f.rgb, a), true);
                    }
                }
                // Wave: letters ride a wave that swells with the beat.
                3 => {
                    let rainbow = look.color >= COLORS.len();
                    let amp = (1.2 + 3.0 * p) * scale;
                    paint_chars(draw, word, wx, wy, s, f.zoom, line_alpha * lit, |ci| {
                        let phase = look.t * 5.0 + (k * 7 + ci) as f32 * 0.55;
                        let rgb = if rainbow { hsv(look.t / 3.0 + (k * 7 + ci) as f32 * 0.04) } else { f.rgb };
                        (phase.sin() * amp, rgb)
                    });
                }
                // Neon: a glow around the letters that flares on the beat.
                4 => {
                    let glow = (0.22 + 0.35 * p) * line_alpha * lit;
                    let o = (0.8 + 0.6 * p) * scale.max(1.0) * 0.6;
                    for (gx, gy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
                        paint_zoom(draw, word, wx + gx, wy + gy, s, f.zoom, argb(f.rgb, glow), false);
                    }
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(mix(0xFFFFFF, f.rgb, 0.25), line_alpha * lit), false);
                }
                // Cinema: a subtitle; words brighten as they're sung.
                5 => {
                    let a = line_alpha * (0.45 + 0.55 * done);
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(mix(0xFFFFFF, f.rgb, 0.35), a), true);
                }
                // Typewriter: letters appear as they're sung, with a blinking cursor.
                6 => {
                    let n = word.chars().count();
                    let vis = typed.saturating_sub(starts[k]).min(n);
                    if vis > 0 {
                        let part: String = word.chars().take(vis).collect();
                        let z = s * f.zoom;
                        let px = wx - (ww * f.zoom - ww) / 2.0;
                        let py = wy - (z - s) * 4.0;
                        paint(draw, &part, px, py, z, argb(f.rgb, line_alpha), true);
                        if blink && typed >= starts[k] && typed < starts[k] + n + 1 && typed < total {
                            let cx2 = px + tw(draw, &part) * z;
                            paint(draw, "_", cx2, py, z, argb(0xFFFFFF, line_alpha), true);
                        }
                    }
                }
                // Fire: flickering flame colours that leap with the beat.
                7 => {
                    paint_chars(draw, word, wx, wy, s, f.zoom, line_alpha * lit, |ci| {
                        let n = (k * 5 + ci) as f32;
                        let heat = (look.t * 7.0 + n * 0.9).sin() * 0.5 + 0.5;
                        let rgb = mix(mix(0xFF3A00, 0xFFE040, heat), 0xFFFFFF, p * 0.5);
                        let lift = -(look.t * 11.0 + n * 1.3).sin().abs() * (0.6 + 2.0 * p) * scale * 0.5;
                        (lift, rgb)
                    });
                }
                // Glitch: split colours that tear apart on the beat.
                8 => {
                    let (nx, ny) = noise(k, look.t);
                    let o = (0.5 + 2.0 * p) * scale * 0.6;
                    let jump = if nx.abs() > 0.85 { nx * 2.0 * scale } else { 0.0 };
                    paint_zoom(draw, word, wx - o + jump, wy, s, f.zoom, argb(0xFF2060, line_alpha * 0.6 * lit), false);
                    paint_zoom(draw, word, wx + o + jump, wy + ny * p * scale, s, f.zoom, argb(0x20E0FF, line_alpha * 0.6 * lit), false);
                    paint_zoom(draw, word, wx + jump, wy, s, f.zoom, argb(mix(f.rgb, 0xFFFFFF, 0.4), line_alpha * lit), false);
                }
                // Rainbow: every letter its own colour, flowing.
                9 => {
                    paint_chars(draw, word, wx, wy, s, f.zoom, line_alpha * lit, |ci| {
                        (0.0, mix(hsv(look.t / 2.0 + (k * 7 + ci) as f32 * 0.06), 0xFFFFFF, p * 0.5))
                    });
                }
                // Outline: a black edge, easy to read anywhere.
                10 => {
                    let o = scale.max(1.0) * 0.5;
                    for (gx, gy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
                        paint_zoom(draw, word, wx + gx, wy + gy, s, f.zoom, argb(0x000000, line_alpha * lit * 0.9), false);
                    }
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha * lit), false);
                }
                // Giant: the word being sung grows big; sung ones stay, the rest wait dim.
                11 => {
                    if singing == Some(k) && tau < lens[k] + 250 {
                        let grow = 1.1 + 0.35 * (-(tau.max(0) as f32) / 400.0).exp();
                        paint_zoom(draw, word, wx, wy, s, f.zoom * grow, argb(f.rgb, line_alpha), true);
                    } else if tau >= 0 {
                        paint_zoom(draw, word, wx, wy, s, f.zoom, argb(mix(f.rgb, 0xAAAAAA, 0.35), line_alpha * 0.8), true);
                    } else {
                        paint_zoom(draw, word, wx, wy, s, 1.0, argb(0x9A9A9A, line_alpha * 0.45), true);
                    }
                }
                // Heartbeat: every word swells and flushes on the beat.
                12 => {
                    let z = f.zoom * (1.0 + 0.3 * p);
                    paint_zoom(draw, word, wx, wy, s, z, argb(mix(f.rgb, 0xFF5577, p * 0.6), line_alpha * lit), true);
                }
                // Rain: letters fall into place as they're sung.
                13 => {
                    let n = word.chars().count().max(1) as i64;
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, _| {
                        let at = onsets[k] + lens[k] * ci as i64 / n;
                        let age = (el - at) as f32;
                        if age < 0.0 {
                            return (0.0, 0.0, f.rgb, line_alpha * 0.12, None);
                        }
                        let fall = (1.0 - (age / 220.0).min(1.0)).powi(2) * 14.0 * scale;
                        (0.0, -fall, f.rgb, line_alpha * (age / 120.0).min(1.0), None)
                    });
                }
                // Disco: every beat each word gets a new colour.
                14 => {
                    let r = ((k as f32 + look.count * 7.0) * 12.9898).sin() * 43758.547;
                    let hue = r - r.floor();
                    let rgb = mix(hsv(hue), 0xFFFFFF, p * 0.6);
                    paint_zoom(draw, word, wx, wy, s, f.zoom * (1.0 + 0.12 * p), argb(rgb, line_alpha * lit), true);
                }
                // Echo: fading copies trail below, longer on the beat.
                15 => {
                    for e in (1..=3).rev() {
                        let d = e as f32 * (1.2 + 3.0 * p) * scale * 0.6;
                        paint_zoom(draw, word, wx, wy + d, s, f.zoom, argb(f.rgb, line_alpha * lit * 0.32 / e as f32), false);
                    }
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha * lit), true);
                }
                // Matrix: waiting letters flicker as code, sung ones settle green.
                16 => {
                    const CODE: [char; 12] = ['ｱ', 'ｲ', 'ｳ', 'ｴ', 'ｵ', 'ｶ', 'ｷ', 'ｸ', '0', '1', 'ﾊ', 'ﾐ'];
                    let n = word.chars().count().max(1) as i64;
                    let tick = (look.t * 12.0) as usize;
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, c| {
                        let at = onsets[k] + lens[k] * ci as i64 / n;
                        let age = el - at;
                        if age < 0 {
                            let g = CODE[(tick + k * 5 + ci * 3) % CODE.len()];
                            (0.0, 0.0, 0x22AA44, line_alpha * 0.45, if c == ' ' { None } else { Some(g) })
                        } else {
                            let rgb = mix(0xFFFFFF, 0x44FF77, (age as f32 / 300.0).min(1.0));
                            (0.0, 0.0, mix(rgb, 0xFFFFFF, p * 0.4), line_alpha, None)
                        }
                    });
                }
                // Explosion: letters fly apart on the beat and snap back.
                17 => {
                    let n = word.chars().count() as f32;
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, _| {
                        let (a, b) = noise(k * 31 + ci, look.count);
                        let dx = (ci as f32 - n / 2.0) * p * 1.6 * scale + a * p * 2.0 * scale;
                        (dx, b * p * 4.0 * scale, f.rgb, line_alpha * lit, None)
                    });
                }
                // Retro: an arcade 3D block shadow.
                18 => {
                    let depth = 3 + (p * 2.0) as i32;
                    for d in (1..=depth).rev() {
                        let o = d as f32 * 0.6 * scale.max(1.0) * 0.6;
                        paint_zoom(draw, word, wx + o, wy + o, s, f.zoom, argb(mix(f.rgb, 0x000000, 0.35 + 0.1 * d as f32), line_alpha * lit), false);
                    }
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(mix(f.rgb, 0xFFFFFF, 0.3), line_alpha * lit), false);
                }
                // Shine: a light sweeps across the line.
                19 => {
                    let sweep = (look.t * 0.45).fract() * 1.6 - 0.3;
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, _| {
                        let at = (starts[k] + ci) as f32 / total as f32;
                        let b = (-((sweep - at) * 9.0).powi(2)).exp();
                        (0.0, 0.0, mix(f.rgb, 0xFFFFFF, b.max(p * 0.5)), line_alpha * lit, None)
                    });
                }
                // Swing: words rock side to side with the tempo.
                20 => {
                    let swing = if look.bpm > 0.0 { look.phase + look.count } else { look.t * 2.0 };
                    let a = (swing * std::f32::consts::PI + k as f32 * 0.7).sin();
                    let dx = a * (1.0 + 2.5 * look.rhythm) * scale;
                    let dy = -a.abs() * (0.5 + 2.0 * p) * scale;
                    paint_zoom(draw, word, wx + dx, wy + dy, s, f.zoom, argb(f.rgb, line_alpha * lit), true);
                }
                // Hologram: see-through cyan that flickers, with a ghost copy and a scan line.
                21 => {
                    let rgb = mix(0x66E0FF, f.rgb, 0.3);
                    let flick = 0.72 + 0.28 * (look.t * 31.0 + ri as f32 * 3.0 + k as f32).sin().abs();
                    let tear = if (look.t * 0.7).fract() < 0.04 { 2.0 * scale } else { 0.0 };
                    paint_zoom(draw, word, wx + 0.7 * scale + tear, wy, s, f.zoom, argb(0x3399FF, line_alpha * 0.28 * lit), false);
                    paint_zoom(draw, word, wx + tear, wy, s, f.zoom, argb(mix(rgb, 0xFFFFFF, p * 0.5), line_alpha * flick * lit * 0.85), false);
                    draw.fill([wx, wy + 8.6 * s, ww, (0.5 * scale).max(0.5)], argb(rgb, 0.22 * line_alpha));
                }
                // Long shadow: a long diagonal shadow behind the letters.
                22 => {
                    for d in (1..=8).rev() {
                        let o = d as f32 * 0.45 * scale.max(1.0) * 0.6;
                        paint_zoom(draw, word, wx + o, wy + o, s, f.zoom, argb(mix(f.rgb, 0x000000, 0.65), line_alpha * lit * 0.5 * (1.0 - d as f32 / 9.0)), false);
                    }
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha * lit), false);
                }
                // Gradient: the colour flows from one end of the line to the other.
                23 => {
                    let other = hsv(look.t * 0.08 + hue + 0.45);
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, _| {
                        let at = (starts[k] + ci) as f32 / total as f32;
                        (0.0, 0.0, mix(mix(f.rgb, other, at), 0xFFFFFF, p * 0.4), line_alpha * lit, None)
                    });
                }
                // Bubbles: every word in its own little pill that lights up when sung.
                24 => {
                    let pad = 1.6 * scale;
                    let (bx, by, bw, bh) = (wx - pad, wy - pad * 0.7, ww * f.zoom + pad * 2.0, 8.0 * s + pad * 1.4);
                    let col = if tau >= 0 { argb(f.rgb, 0.42 * line_alpha) } else { argb(0x000000, 0.38 * line_alpha) };
                    draw.fill([bx + pad * 0.5, by, bw - pad, bh], col);
                    draw.fill([bx, by + pad * 0.5, pad * 0.5, bh - pad], col);
                    draw.fill([bx + bw - pad * 0.5, by + pad * 0.5, pad * 0.5, bh - pad], col);
                    let txt = if tau >= 0 { mix(0xFFFFFF, f.rgb, 0.15) } else { 0xD8D8D8 };
                    paint_zoom(draw, word, wx, wy, s, f.zoom * (1.0 + 0.1 * p), argb(txt, line_alpha), true);
                }
                // Ice: frosty blue letters that twinkle.
                25 => {
                    let o = scale.max(1.0) * 0.45;
                    for (gx, gy) in [(-o, 0.0), (o, 0.0), (0.0, o)] {
                        paint_zoom(draw, word, wx + gx, wy + gy, s, f.zoom, argb(0x1F4E8C, line_alpha * lit * 0.6), false);
                    }
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, _| {
                        let b = (look.t * 4.0 + (k * 7 + ci) as f32 * 2.3).sin().max(0.0).powi(8);
                        (0.0, 0.0, mix(mix(0x9FDFFF, f.rgb, 0.15), 0xFFFFFF, b.max(p * 0.5)), line_alpha * lit, None)
                    });
                }
                // Gold: shining gold letters with a dark edge.
                26 => {
                    let o = scale.max(1.0) * 0.5;
                    for (gx, gy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
                        paint_zoom(draw, word, wx + gx, wy + gy, s, f.zoom, argb(0x3A2200, line_alpha * lit * 0.85), false);
                    }
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, _| {
                        let at = (starts[k] + ci) as f32 / total as f32;
                        let sh = ((look.t * 3.0 - at * 8.0).sin() + 1.0) / 2.0;
                        (0.0, 0.0, mix(mix(0xB8860B, 0xFFE38A, sh), 0xFFFFFF, p * 0.5), line_alpha * lit, None)
                    });
                }
                // Ink: copies of each word come together as it's sung.
                27 => {
                    if tau < 0 {
                        paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha * 0.12), false);
                    } else {
                        let age = tau as f32;
                        let spread = (1.0 - age / 420.0).max(0.0).powi(2) * 6.0 * scale;
                        if spread > 0.05 {
                            for (gx, gy) in [(-1.0, -0.5), (1.0, 0.4), (-0.4, 1.0), (0.5, -1.0)] {
                                paint_zoom(draw, word, wx + gx * spread, wy + gy * spread, s, f.zoom, argb(f.rgb, line_alpha * 0.25), false);
                            }
                        }
                        paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha * (age / 300.0).min(1.0)), true);
                    }
                }
                // Earthquake: the whole line shakes with the beat.
                28 => {
                    let (nx, ny) = noise(1000 + ri, look.t * 2.0);
                    let amp = (0.4 + 3.2 * look.pulse(beat).max(p)) * scale;
                    paint_zoom(draw, word, wx + nx * amp, wy + ny * amp, s, f.zoom, argb(f.rgb, line_alpha * lit), true);
                }
                // Spiral: letters spin into place as they're sung.
                29 => {
                    let n = word.chars().count().max(1) as i64;
                    paint_chars4(draw, word, wx, wy, s, f.zoom, |ci, _| {
                        let at = onsets[k] + lens[k] * ci as i64 / n;
                        let age = (el - at) as f32;
                        if age < 0.0 {
                            return (0.0, 0.0, f.rgb, line_alpha * 0.12, None);
                        }
                        let r = (1.0 - age / 520.0).max(0.0).powi(2) * 11.0 * scale;
                        let a = age / 70.0 + ci as f32;
                        (a.cos() * r, a.sin() * r, f.rgb, line_alpha * (age / 160.0).min(1.0), None)
                    });
                }
                // Fly in: each word arrives big and settles when it's sung.
                30 => {
                    if tau >= 0 {
                        let age = tau as f32;
                        let grow = 1.0 + 2.2 * (1.0 - age / 260.0).max(0.0).powi(2);
                        paint_zoom(draw, word, wx, wy, s, f.zoom * grow, argb(f.rgb, line_alpha * (age / 140.0).min(1.0)), true);
                    } else {
                        paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha * 0.1), false);
                    }
                }
                // Ghost: sung words leave a ghost that floats up.
                31 => {
                    if tau >= 0 {
                        let age = tau as f32;
                        let rise = (age / 1000.0).min(1.5) * 6.0 * scale;
                        let ga = 0.4 * (1.0 - age / 1500.0).max(0.0);
                        if ga > 0.01 {
                            paint_zoom(draw, word, wx, wy - rise, s, f.zoom * (1.0 + age / 3000.0), argb(mix(f.rgb, 0xFFFFFF, 0.6), line_alpha * ga), false);
                        }
                    }
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(mix(f.rgb, 0xFFFFFF, 0.3), line_alpha * lit * 0.9), true);
                }
                // Sticker: a thick white border like a sticker.
                32 => {
                    let o = scale.max(1.0) * 0.6;
                    for (gx, gy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o), (-o, -o), (o, -o), (-o, o), (o, o)] {
                        paint_zoom(draw, word, wx + gx, wy + gy, s, f.zoom, argb(0xFFFFFF, line_alpha * lit), false);
                    }
                    paint_zoom(draw, word, wx + o * 0.7, wy + o * 1.6, s, f.zoom, argb(0x000000, line_alpha * lit * 0.25), false);
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(mix(f.rgb, 0x000000, 0.25), line_alpha * lit), false);
                }
                // Pulse: the whole line changes colour on every beat.
                33 => {
                    let rgb = mix(hsv(look.count * 0.13 + hue), 0xFFFFFF, look.tempo.max(p) * 0.5);
                    paint_zoom(draw, word, wx, wy, s, f.zoom * (1.0 + 0.1 * look.tempo * look.rhythm.max(0.3)), argb(rgb, line_alpha * lit), true);
                }
                // Classic: the whole line, gently pulsing.
                _ => {
                    paint_zoom(draw, word, wx, wy, s, f.zoom, argb(f.rgb, line_alpha), true);
                }
            }
            x += slot + space * s;
        }
    }
}

/// A line fixed in the world like a sign: each word sits at its own point along `along`, so
/// seen from the side the far words are smaller and closer together.
#[allow(clippy::too_many_arguments)]
fn render_fixed(
    host: &dyn ClientHost,
    draw: &mut dyn DrawContext,
    text: &str,
    centre: [f64; 3],
    along: [f64; 3],
    scale: f32,
    alpha: f32,
    look: &Look,
    hue: f32,
    elapsed: Option<i64>,
    sung: i64,
    timing: &[i64],
    [w, h]: [f32; 2],
    eye: [f64; 3],
) {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return;
    }
    let widths: Vec<f32> = words.iter().map(|x| tw(draw, x)).collect();
    let space = tw(draw, " ");
    let total_w: f32 = widths.iter().sum::<f32>() + space * (words.len() - 1) as f32;
    // How many blocks one text pixel takes, so words on screen are as big as their spacing: from
    // how many screen pixels half a block takes where the line hangs.
    let dc = ((centre[0] - eye[0]).powi(2) + (centre[1] - eye[1]).powi(2) + (centre[2] - eye[2]).powi(2)).sqrt().max(0.5);
    let ppb = match (host.project(centre), host.project([centre[0], centre[1] + 0.5, centre[2]])) {
        (Some(a), Some(b)) => (((a[1] - b[1]) / 2.0 * h) as f64 / 0.5).abs(),
        _ => 0.0,
    };
    let unit = if ppb > 1.0 { (scale as f64 * 4.0 / dc) / ppb } else { scale as f64 * 0.0125 };
    let chars: usize = words.iter().map(|x| x.chars().count() + 1).sum::<usize>().max(1);
    let mut acc = 0usize;
    let mut x = -total_w / 2.0;
    let el = elapsed.unwrap_or(i64::MAX / 4);
    for (k, word) in words.iter().enumerate() {
        let mid = (x + widths[k] / 2.0) as f64 * unit;
        x += widths[k] + space;
        let onset = if timing.len() == words.len() { timing[k].max(0) } else { sung * acc as i64 / chars as i64 };
        let next = if timing.len() == words.len() {
            timing.get(k + 1).copied().unwrap_or(sung.max(onset + 300))
        } else {
            sung * (acc + word.chars().count() + 1) as i64 / chars as i64
        };
        acc += word.chars().count() + 1;
        let p = [centre[0] + along[0] * mid, centre[1], centre[2] + along[2] * mid];
        let Some([px, py, pz]) = host.project(p) else { continue };
        if !(-1.0..=1.0).contains(&pz) {
            continue;
        }
        let d = ((p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2)).sqrt().max(0.5);
        let s = (scale * 4.0 / d as f32).clamp(0.2, 10.0);
        let sx = (px + 1.0) / 2.0 * w;
        let sy = (1.0 - py) / 2.0 * h;
        if sx < -w * 0.5 || sx > w * 1.5 || sy < -h * 0.5 || sy > h * 1.5 {
            continue;
        }
        // Each word keeps its own karaoke timing.
        let e = elapsed.map(|_| el - onset);
        render_line(draw, word, sx, sy, s, alpha, look, hue + k as f32 * 0.05, e, (next - onset).max(120), w * 4.0, &[0]);
    }
}

impl Musica {
    fn my_screen(&self, host: &dyn ClientHost, screen: &ScreenView<'_>) -> bool {
        screen.id == "mod" && host.mod_screen() == Some(ID)
    }

    fn draw(&self, host: &dyn ClientHost, draw: &mut dyn DrawContext) {
        let mut player = lock(&self.player);
        player.follow_start(host);
        player.follow_pause(host);
        let [w, h] = draw.size();
        set_bold(false);
        set_ttf(0);
        set_lyric_alpha((host.setting_f64("lyrics_opacity").unwrap_or(100.0) / 100.0) as f32);

        if let Some((text, left)) = &player.toast {
            let a = (*left as f32 / 15.0).min(1.0);
            scaled_text(draw, text, w / 2.0, h * 0.30, 1.0, argb(0xFFFF55, a), true);
        }
        let Some(p) = &player.now else { return };
        let now = host.millis();
        let raw = p.ms(now);
        let paused = p.pause_at.is_some();
        let waiting = p.start.is_none();
        let duration = p.song.duration;
        let name = p.song.name.clone();
        let energy = p.song.energy.clone();
        let style_pick = p.style;
        let beat = energy.as_ref().map(|r| r.beat(raw));
        let ttf_pick = ttf_setting(host);
        // A real font has no fancy Unicode letters: the letter style is left plain with it.
        let font = if ttf_pick > 0 { 0 } else { (host.setting_choice("lyrics_font").unwrap_or(0) as usize).min(FONT_KEYS.len() - 1) };
        let make_look = |host: &dyn ClientHost| {
            let mut l = match beat {
                Some(b) => Look::from(host).with_beat(b),
                None => Look::from(host),
            };
            if let Some(st) = style_pick {
                l.style = st;
            }
            l
        };

        // Now playing: a small box in a top corner, with a progress bar and little bars.
        let beat_now = energy.as_deref().map(|e| (e.level.as_slice(), (raw / 50) as usize));
        if host.setting_bool("widget").unwrap_or(true) {
            let title = format!("{} {name}", if paused { "⏸" } else { "♪" });
            let time = if waiting {
                "Cargando…".to_string()
            } else {
                let t = match duration {
                    Some(d) => format!("{} / {}", clock(raw), clock(d)),
                    None => clock(raw),
                };
                match beat.filter(|b| b.bpm > 0.0 && !paused) {
                    Some(b) => format!("{t}  {:.0} BPM", b.bpm),
                    None => t,
                }
            };
            let bars_w = if beat_now.is_some() && !waiting && !paused { 30.0 } else { 0.0 };
            let tw = (tw(draw, &title).max(tw(draw, &time) + bars_w) + 10.0).clamp(90.0, (w - 4.0).max(90.0));
            let mut right = host.setting_choice("widget_corner").unwrap_or(0) == 1;
            let mut hidden = false;
            if host.setting_bool("dodge_f3").unwrap_or(true) {
                let corner = |r: bool| if r { mod_api::Corner::TopRight } else { mod_api::Corner::TopLeft };
                if !host.native_corner_shown(corner(right)) {
                    if host.native_corner_shown(corner(!right)) { right = !right } else { hidden = true }
                }
            }
            let x = if right { w - tw - 2.0 } else { 2.0 };
            if !hidden {
                draw.fill_gradient([x, 2.0, tw, 28.0], 0xC0201030, 0xC0100818);
                draw.fill([x, 2.0, 2.0, 28.0], 0xFFB36BFF);
                draw.text(&title, [x + 5.0, 5.0], 0xFFFFFFFF, true);
                draw.text(&time, [x + 5.0, 15.0], 0xFFBBAADD, true);
                if let Some(d) = duration.filter(|d| *d > 0) {
                    let f = (raw as f32 / d as f32).clamp(0.0, 1.0);
                    draw.fill([x + 5.0, 25.0, tw - 10.0, 2.0], 0x60FFFFFF);
                    draw.fill([x + 5.0, 25.0, (tw - 10.0) * f, 2.0], 0xFFB36BFF);
                }
                if let Some((e, i)) = beat_now.filter(|_| bars_w > 0.0) {
                    for b in 0..7usize {
                        let v = e.get(i.saturating_sub(b * 2)).copied().unwrap_or(0) as f32 / 9.0;
                        let bh = 1.0 + v * 8.0;
                        let col = hsv(b as f32 * 0.08 + now as f32 / 6000.0);
                        draw.fill([x + tw - 6.0 - (b as f32 + 1.0) * 3.5, 23.0 - bh, 2.5, bh], argb(col, 0.9));
                    }
                }
            }
        }
        // The big bars: a ripple from the middle, the music's loudness spreading out.
        let vis = host.setting_choice("visualizer").unwrap_or(2);
        if let Some((e, i)) = beat_now.filter(|_| vis > 0 && !waiting) {
            let n = 25usize;
            let bw = 4.0;
            let total = n as f32 * (bw + 1.0);
            let x0 = w / 2.0 - total / 2.0;
            let look = Look::from(host);
            for b in 0..n {
                let d = (b as f32 - (n as f32 - 1.0) / 2.0).abs() as usize;
                let v = e.get(i.saturating_sub(d)).copied().unwrap_or(0) as f32 / 9.0;
                let bh = 1.0 + v * 14.0;
                let y = if vis == 1 { 4.0 } else { h - 58.0 - bh };
                draw.fill([x0 + b as f32 * (bw + 1.0), y, bw, bh], argb(look.rgb(b as f32 * 0.03), 0.75));
            }
        }

        // A banner with the song's name when it starts, and how sure we are it's the right song.
        let info = p.song.info.clone();
        if host.setting_bool("banner").unwrap_or(true) && !waiting && raw < 4200 {
            let a = (raw as f32 / 400.0).min(1.0).min(((4200 - raw) as f32 / 700.0).clamp(0.0, 1.0));
            set_bold(font == 1);
            let look = make_look(host);
            scaled_text(draw, "♪ Ahora suena ♪", w / 2.0, h * 0.16, 1.0, argb(0xCCBBEE, a), true);
            prepare_ttf(host, ttf_pick, &name);
            set_ttf(ttf_pick);
            render_line(draw, &styled(&name, font), w / 2.0, h * 0.16 + 20.0, 2.0, a, &look, 0.0, Some(raw), 1600, w * 0.9, &[]);
            set_ttf(0);
            set_bold(false);
        }
        if host.setting_bool("diagnostic").unwrap_or(false) {
            let slot = p.song.slot;
            let lyr = p.song.lyrics.as_ref();
            let off = player.offset_of(host, slot);
            let ms = raw + off;
            let cur = lyr.and_then(|l| l.current(ms));
            let state = match p.sound.and_then(|s| s.state(host)) {
                Some(SoundState::Playing) => "sonando",
                Some(SoundState::Waiting) => "esperando",
                Some(SoundState::Stopped) => "parado",
                _ => "?",
            };
            let real_words = lyr.is_some_and(|l| l.lines.iter().any(|x| !x.words.is_empty()));
            let mut rows = vec![
                format!("Canción {slot} · {}", clock(raw)),
                match &info {
                    Some(i) => format!("Fuente: {}", if i.source.is_empty() { "?" } else { &i.source }),
                    None => "Fuente: -".into(),
                },
                format!("Tipo: {} · palabras: {}", info.as_ref().map(|i| i.kind.as_str()).filter(|k| !k.is_empty()).unwrap_or("?"), if real_words { "con tiempos reales" } else if lyr.is_some() { "estimadas por sílabas y ritmo" } else { "-" }),
                format!(
                    "Línea {}/{} · desfase {:+} ms (general {:+}, canción {:+}, archivo {:+})",
                    cur.map(|c| c + 1).unwrap_or(0),
                    lyr.map(|l| l.lines.len()).unwrap_or(0),
                    off + lyr.map(|l| l.offset).unwrap_or(0),
                    host.setting_f64("lyrics_offset").unwrap_or(0.0) as i64,
                    player.song_offset.get(&slot).copied().unwrap_or(0),
                    lyr.map(|l| l.offset).unwrap_or(0)
                ),
                format!("Sonido: {state} · pausa: {} · juego pausado: {}", if paused { "sí" } else { "no" }, if host.is_paused() { "sí" } else { "no" }),
            ];
            if let Some(i) = info.as_ref().filter(|i| !i.identified.is_empty()) {
                rows.push(format!("Identificada: {}", i.identified));
            }
            if info.as_ref().is_some_and(|i| i.locked) {
                rows.push("Bloqueada: no se vuelve a descargar".into());
            }
            let y0 = if host.setting_bool("widget").unwrap_or(true) { 34.0 } else { 4.0 };
            let bw = rows.iter().map(|r| tw(draw, r)).fold(0.0f32, f32::max) * 0.75 + 8.0;
            draw.fill([2.0, y0, bw, rows.len() as f32 * 8.0 + 4.0], 0xB0000000);
            for (k, r) in rows.iter().enumerate() {
                paint(draw, r, 6.0, y0 + 2.0 + k as f32 * 8.0, 0.75, if k == 1 { argb(info.as_ref().map(|i| i.colour()).unwrap_or(0xAAAAAA), 1.0) } else { 0xFFDDDDDD }, false);
            }
        }

        if NO_EXTRAS && !waiting && !paused {
            edge_flash(draw, &make_look(host), w, h);
        }

        if !host.setting_bool("lyrics").unwrap_or(true) || waiting {
            player.floating.clear();
            player.placed = None;
            return;
        }
        // A song without lyrics shows its name instead, moving to the music.
        let lyrics = match p.song.lyrics.clone() {
            Some(l) => l,
            None if host.setting_bool("title_no_lyrics").unwrap_or(true) => {
                let text = if p.song.instrumental() { format!("♪ {name} · instrumental ♪") } else { format!("♪ {name} ♪") };
                Lyrics { lines: vec![Line { at: 0, text, words: Vec::new() }], ..Default::default() }
            }
            None => return,
        };
        let slot = p.song.slot;
        // The offset settings: positive shows the words sooner.
        let ms = raw + player.offset_of(host, slot);
        let scale = host.setting_f64("lyrics_size").unwrap_or(2.0) as f32;
        let look = make_look(host);
        let lyrics = styled_lyrics(&lyrics, font);
        set_bold(font == 1);
        if ttf_pick > 0 {
            if let Some(i) = lyrics.current(ms) {
                let (g0, g1) = lyrics.group(i);
                for k in g0.saturating_sub(1)..=(g1 + 1).min(lyrics.lines.len().saturating_sub(1)) {
                    prepare_ttf(host, ttf_pick, &lyrics.lines[k].text);
                }
            }
            for f in &player.floating {
                if let Some(l) = lyrics.lines.get(f.line) {
                    prepare_ttf(host, ttf_pick, &l.text);
                }
            }
            prepare_ttf(host, ttf_pick, " ");
        }
        set_ttf(ttf_pick);
        let current = lyrics.current(ms);
        let world = host.setting_choice("lyrics_mode").unwrap_or(0) == 0;
        let camera = if world { host.camera() } else { None };

        if let Some(camera) = camera {
            self.draw_world(host, draw, &mut player, &camera, &lyrics, slot, current, ms, scale, &look, energy.as_deref());
            return;
        }
        player.floating.clear();
        player.placed = None;

        // On screen: the current line (all of them when several are sung together), with the
        // last and next small around it.
        let Some(i) = current else { return };
        let (g0, g1) = lyrics.group(i);
        let line = &lyrics.lines[g0];
        if line.text.trim().is_empty() {
            return;
        }
        let since = ms - line.at;
        let rise = (1.0 - (since as f32 / 300.0).clamp(0.0, 1.0)) * 6.0;
        let rows_of = |draw: &mut dyn DrawContext, t: &str| ((tw(draw, t) * scale) / (w * 0.9)).ceil().max(1.0);
        let mut heights = Vec::new();
        for k in g0..=g1 {
            heights.push(rows_of(draw, &lyrics.lines[k].text) * 11.0 * scale);
        }
        let gap = 3.0 * scale;
        let block = heights.iter().sum::<f32>() + gap * (heights.len() as f32 - 1.0);
        let centre = match host.setting_choice("lyrics_place").unwrap_or(2) {
            0 => 36.0 + block / 2.0,
            1 => h / 2.0 - 20.0,
            _ => h - 66.0 - block / 2.0,
        };
        let context = host.setting_bool("lyrics_context").unwrap_or(true);
        if context && g0 > 0 && !lyrics.lines[g0 - 1].text.trim().is_empty() {
            scaled_text(draw, &lyrics.lines[g0 - 1].text, w / 2.0, centre - block / 2.0 - 13.0, 1.0, argb(0xBBBBBB, 0.5 * lyric_alpha()), true);
        }
        let mut y = centre - block / 2.0;
        for (n, k) in (g0..=g1).enumerate() {
            let l = &lyrics.lines[k];
            let timing = word_timing(&lyrics, k, energy.as_deref());
            let hh = heights[n];
            // A second voice: another hue so you can tell who sings.
            render_line(draw, &l.text, w / 2.0, y + hh / 2.0 + rise, scale, 1.0, &look, n as f32 * 0.33, Some(ms - l.at), sung_span(&lyrics, k), w * 0.9, &timing);
            y += hh + gap;
        }
        if NO_EXTRAS {
            let spread = (tw(draw, &line.text) * scale).min(w * 0.9);
            sparks(draw, &look, w / 2.0, centre, spread, scale.min(2.0));
        }
        if context {
            if let Some(next) = lyrics.lines.get(g1 + 1).filter(|l| !l.text.trim().is_empty()) {
                scaled_text(draw, &next.text, w / 2.0, centre + block / 2.0 + 4.0, 1.0, argb(0xBBBBBB, 0.45 * lyric_alpha()), true);
            }
        }
    }

    /// Lyrics hanging in the world: each line appears in front of where you look, a few
    /// blocks away, stays where it appeared (it gets bigger as you walk up to it), rises
    /// slowly and fades out after the next lines come.
    #[allow(clippy::too_many_arguments)]
    fn draw_world(
        &self,
        host: &dyn ClientHost,
        draw: &mut dyn DrawContext,
        player: &mut Player,
        camera: &Camera,
        lyrics: &Lyrics,
        slot: u32,
        current: Option<usize>,
        ms: i64,
        scale: f32,
        look: &Look,
        energy: Option<&Rhythm>,
    ) {
        let [w, h] = draw.size();
        let keep = host.setting_f64("lyrics_lines").unwrap_or(3.0).clamp(1.0, 6.0) as usize;
        // A new line: retire the old ones and place it in front of the camera.
        if let Some(i) = current {
            if player.placed != Some((slot, i)) {
                player.placed = Some((slot, i));
                for f in &mut player.floating {
                    f.replaced.get_or_insert(ms);
                }
                let (g0, g1) = lyrics.group(i);
                if !lyrics.lines[i].text.trim().is_empty() {
                    let distance = host.setting_f64("lyrics_distance").unwrap_or(5.0);
                    let layout = host.setting_choice("world_layout").unwrap_or(0) as usize;
                    let facing = host.setting_choice("world_facing").unwrap_or(0) as usize;
                    let yaw = (camera.yaw as f64).to_radians();
                    let forward = [-yaw.sin(), 0.0, yaw.cos()];
                    let right = [-yaw.cos(), 0.0, -yaw.sin()];
                    let n = player.spawned;
                    player.spawned = player.spawned.wrapping_add(1);
                    let r1 = (player.random() % 1000) as f64 / 1000.0;
                    let r2 = (player.random() % 1000) as f64 / 1000.0;
                    let r3 = (player.random() % 1000) as f64 / 1000.0;
                    let c = camera.position;
                    // A point around you: `angle` radians from where you look (+ to the right).
                    let around = |angle: f64, d: f64, up: f64| {
                        let (sa, ca) = angle.sin_cos();
                        let dir = [forward[0] * ca + right[0] * sa, 0.0, forward[2] * ca + right[2] * sa];
                        ([c[0] + dir[0] * d, c[1] + up, c[2] + dir[2] * d], dir)
                    };
                    let d = distance * (0.9 + r1 * 0.2);
                    // Each line kept on screen gets its own height, so lines never land on each other.
                    let row = (scale as f64 * 0.5 + 0.3) * d / 5.0;
                    let slot_up = 0.7 - (n % keep.max(1) as u32) as f64 * row;
                    let (mut pos, mut dir) = match layout {
                        // To one side: you see it turning your head a little.
                        1 | 2 => {
                            let sign = if layout == 1 { -1.0 } else { 1.0 };
                            around(sign * (0.70 + r1 * 0.12), d, slot_up + r2 * 0.1)
                        }
                        // All around you, as if the lines circled you.
                        3 => around(n as f64 * 2.399 + r1 * 0.5, d, -0.2 + r2 * 0.9),
                        // High up, like a sky banner.
                        4 => around((r1 - 0.5) * 0.9, d * 0.8, 2.2 + r2 * 1.2),
                        // Anywhere around: in front, beside, behind, up or down.
                        6 => around(r1 * std::f64::consts::TAU, d * (0.7 + r3 * 0.6), -0.6 + r2 * 2.6),
                        // In front: centre, left, right in turn, a little up or down, near or far.
                        _ => {
                            let side = if keep == 1 { 0.0 } else { [0.0, -1.0, 1.0][(n % 3) as usize] * (0.8 + r1 * 1.2) * distance / 5.0 };
                            let up = if keep == 1 { -0.1 + r2 * 0.8 } else { slot_up + r2 * 0.1 };
                            ([c[0] + forward[0] * d + right[0] * side, c[1] + up, c[2] + forward[2] * d + right[2] * side], forward)
                        }
                    };
                    // The line runs across your view (it faces you from where it appeared).
                    let mut along = [-dir[2], 0.0, dir[0]];
                    if along[0] * right[0] + along[2] * right[2] < 0.0 {
                        along = [-along[0], 0.0, -along[2]];
                    }
                    // On the map: on the wall, the ground or whatever block you look at.
                    if layout == 5 {
                        if let Some(hit) = host.raycast((distance * 3.0).clamp(8.0, 40.0) as f32) {
                            let fx = hit.x - hit.block_x as f64;
                            let fy = hit.y - hit.block_y as f64;
                            let fz = hit.z - hit.block_z as f64;
                            let e = 0.002;
                            let normal = if fx < e {
                                [-1.0, 0.0, 0.0]
                            } else if fx > 1.0 - e {
                                [1.0, 0.0, 0.0]
                            } else if fz < e {
                                [0.0, 0.0, -1.0]
                            } else if fz > 1.0 - e {
                                [0.0, 0.0, 1.0]
                            } else if fy > 1.0 - e {
                                [0.0, 1.0, 0.0]
                            } else {
                                [0.0, -1.0, 0.0]
                            };
                            let stack = (n % keep.max(1) as u32) as f64 * 0.55 * scale as f64 / 2.0;
                            if normal[1] == 0.0 {
                                // A wall: like a sign stuck on it, the newest on top.
                                pos = [hit.x + normal[0] * 0.08, hit.y + 0.3 - stack, hit.z + normal[2] * 0.08];
                                along = [-normal[2], 0.0, normal[0]];
                                if along[0] * right[0] + along[2] * right[2] < 0.0 {
                                    along = [-along[0], 0.0, -along[2]];
                                }
                            } else if normal[1] > 0.0 {
                                // The ground: standing on it.
                                pos = [hit.x, hit.y + 0.55 + stack, hit.z];
                            } else {
                                pos = [hit.x, hit.y - 0.5 - stack, hit.z];
                            }
                            dir = [0.0; 3];
                        }
                    }
                    let _ = dir;
                    // Seen from the side: the line turns away from you.
                    if facing == 2 {
                        let a: f64 = if n % 2 == 0 { 0.95 } else { -0.95 };
                        let (sa, ca) = a.sin_cos();
                        along = [along[0] * ca - along[2] * sa, 0.0, along[0] * sa + along[2] * ca];
                    }
                    let fixed = (facing != 0 || layout == 5).then_some(along);
                    // Lines sung together hang one under the other.
                    for (m, k) in (g0..=g1).enumerate() {
                        let p2 = [pos[0], pos[1] - m as f64 * 0.5 * scale as f64 / 2.0, pos[2]];
                        player.floating.push(Floating {
                            line: k,
                            pos: p2,
                            born: ms,
                            replaced: None,
                            old_since: None,
                            hue: n as f32 * 0.17 + m as f32 * 0.33,
                            along: fixed,
                        });
                    }
                }
            }
        }
        // An old line leaves quickly, so it never sits under the new one.
        const FADE_OUT: i64 = 350;
        // The last `keep` lines stay; older ones fade out.
        let len = player.floating.len();
        for (k, f) in player.floating.iter_mut().enumerate() {
            if len - k > keep {
                f.old_since.get_or_insert(ms);
            }
        }
        player
            .floating
            .retain(|f| !f.old_since.is_some_and(|o| ms - o > FADE_OUT) && ms >= f.born - 100 && f.line < lyrics.lines.len());
        if player.floating.len() > keep + 4 {
            let extra = player.floating.len() - keep - 4;
            player.floating.drain(..extra);
        }

        // Farthest first, so nearer lines draw over them.
        let c = camera.position;
        let dist = |p: [f64; 3]| ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt();
        let len = player.floating.len();
        let mut order: Vec<usize> = (0..len).collect();
        order.sort_by(|&a, &b| dist(player.floating[b].pos).total_cmp(&dist(player.floating[a].pos)));
        for k in order {
            let f = &player.floating[k];
            let age = (ms - f.born).max(0) as f64 / 1000.0;
            let sway = (age * 1.3 + f.hue as f64 * 9.0).sin() * 0.05;
            let pos = [f.pos[0], f.pos[1] + age.min(15.0) * 0.06 + sway, f.pos[2]];
            let d = dist(pos).max(0.5);
            let Some([x, y, z]) = host.project(pos) else { continue };
            if !(-1.0..=1.0).contains(&z) {
                continue;
            }
            let sx = (x + 1.0) / 2.0 * w;
            let sy = (1.0 - y) / 2.0 * h;
            if sx < -w || sx > 2.0 * w || sy < -h || sy > 2.0 * h {
                continue;
            }
            // Perspective: as big as `scale` at 4 blocks.
            let s = (scale * 4.0 / d as f32).clamp(0.25, 10.0);
            let fade_in = ((ms - f.born) as f32 / 250.0).clamp(0.0, 1.0);
            let alpha = match (f.replaced, f.old_since) {
                (None, None) => fade_in,
                (_, Some(o)) => 0.7 * (1.0 - (ms - o) as f32 / FADE_OUT as f32).clamp(0.0, 1.0),
                (Some(_), None) => 0.7,
            };
            let line = &lyrics.lines[f.line];
            let elapsed = if f.replaced.is_some() { None } else { Some(ms - line.at) };
            let timing = word_timing(lyrics, f.line, energy);
            if let Some(along) = f.along {
                render_fixed(host, draw, &line.text, pos, along, scale, alpha, look, f.hue, elapsed, sung_span(lyrics, f.line), &timing, [w, h], c);
            } else {
                // Kept whole on the screen: a long word makes the line smaller, and a line that
                // would run past an edge is pushed back in.
                let edge = 6.0;
                let room = (w - edge * 2.0).max(40.0);
                let longest = line.text.split_whitespace().map(|x| tw(draw, x)).fold(0.0f32, f32::max).max(1.0);
                let s = s.min(room / longest);
                let max = (230.0 * s).min(room);
                let bw = (tw(draw, &line.text) * s).min(max);
                // To one side: the line starts at its point and runs inward (left side to the right,
                // right side to the left), so it reads from where it sits and never leaves the screen.
                let sx = match host.setting_choice("world_layout").unwrap_or(0) {
                    1 => sx + bw / 2.0,
                    2 => sx - bw / 2.0,
                    _ => sx,
                };
                let sx = if sx + bw / 2.0 > 0.0 && sx - bw / 2.0 < w { sx.clamp(edge + bw / 2.0, w - edge - bw / 2.0) } else { sx };
                render_line(draw, &line.text, sx, sy, s, alpha, look, f.hue, elapsed, sung_span(lyrics, f.line), max, &timing);
            }
        }
    }
}

// ---------------------------------------------------------------- menu

mod menu {
    use super::*;
    use mod_api::client::widgets::{Button, Outcome, TextField, WidgetId, Widgets};

    /// What a button of the menu does.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Act {
        /// The song in row `n` of the list (from the scroll).
        Row(usize),
        Up,
        Down,
        Prev,
        Toggle,
        Next,
        Tab(usize),
        // Música
        VolDown,
        VolUp,
        SyncDown,
        SyncUp,
        SongSyncDown,
        SongSyncUp,
        Shuffle,
        Repeat,
        Widget,
        Corner,
        Crossfade,
        Banner,
        MuteGame,
        // Letras
        Lyrics,
        Mode,
        SizeDown,
        SizeUp,
        AlphaDown,
        AlphaUp,
        DistDown,
        DistUp,
        LinesDown,
        LinesUp,
        Place,
        Context,
        Layout,
        Facing,
        // Estilo
        StyleDown,
        StyleUp,
        ColorDown,
        ColorUp,
        FontDown,
        FontUp,
        TtfDown,
        TtfUp,
        Boxed,
        // Ritmo
        FxDown,
        FxUp,
        SourceDown,
        SourceUp,
        RhythmDown,
        RhythmUp,
        Visualizer,
        Reset,
        Close,
    }

    impl Act {
        /// The tab it lives on, or None when it always shows.
        fn tab(self) -> Option<usize> {
            use Act::*;
            match self {
                VolDown | VolUp | SyncDown | SyncUp | SongSyncDown | SongSyncUp | Shuffle | Repeat | Widget | Corner | Crossfade | Banner | MuteGame => Some(0),
                Lyrics | Mode | SizeDown | SizeUp | AlphaDown | AlphaUp | DistDown | DistUp | LinesDown | LinesUp | Place | Context | Layout | Facing => Some(1),
                StyleDown | StyleUp | ColorDown | ColorUp | FontDown | FontUp | TtfDown | TtfUp | Boxed => Some(2),
                FxDown | FxUp | SourceDown | SourceUp | RhythmDown | RhythmUp | Visualizer => Some(3),
                _ => None,
            }
        }
    }

    const ROW_H: f32 = 18.0;
    const STEP: f32 = 20.0;
    const GAP: f32 = 2.0;
    const OFF: [f32; 4] = [-5000.0, -5000.0, 1.0, 1.0];
    const TABS: [&str; 4] = ["Música", "Letras", "Estilo", "Ritmo"];
    const NTABS: usize = 4;
    const TAB_ICONS: [&str; 4] = ["♫", "✎", "✦", "♥"];

    /// The menu's colours (THEME_NAMES): border, background top and bottom, header top and
    /// bottom, accent, title text, soft text.
    #[derive(Clone, Copy)]
    struct Theme {
        border: u32,
        top: u32,
        bottom: u32,
        head_top: u32,
        head_bottom: u32,
        accent: u32,
        title: u32,
        soft: u32,
        back: u32,
    }
    const THEMES: [Theme; 7] = [
        Theme { border: 0xFF5A3A86, top: 0xE81A1029, bottom: 0xE80C0814, head_top: 0xFF5B3490, head_bottom: 0xFF2E1A4A, accent: 0xFFB36BFF, title: 0xFFFFE9A8, soft: 0xFFBBAADD, back: 0x140A28 },
        Theme { border: 0xFF2A5F8F, top: 0xE80B1C2E, bottom: 0xE8050C16, head_top: 0xFF1F6FA8, head_bottom: 0xFF0F3556, accent: 0xFF4FD2FF, title: 0xFFD8F6FF, soft: 0xFF9CC4DD, back: 0x061A2C },
        Theme { border: 0xFF8F4A2A, top: 0xE82A1410, bottom: 0xE8140808, head_top: 0xFFC2562F, head_bottom: 0xFF6A2A1A, accent: 0xFFFFA04A, title: 0xFFFFF0C8, soft: 0xFFE8B9A0, back: 0x2A1008 },
        Theme { border: 0xFF3D7A3A, top: 0xE8102214, bottom: 0xE8060E08, head_top: 0xFF3E8E3A, head_bottom: 0xFF1D4A1C, accent: 0xFF7CFF6B, title: 0xFFE8FFD8, soft: 0xFFA9D6A0, back: 0x0A1E0C },
        Theme { border: 0xFF555555, top: 0xF01A1A1A, bottom: 0xF00A0A0A, head_top: 0xFF3A3A3A, head_bottom: 0xFF222222, accent: 0xFFE6E6E6, title: 0xFFFFFFFF, soft: 0xFFB0B0B0, back: 0x101010 },
        Theme { border: 0xFFB0508A, top: 0xE82A1024, bottom: 0xE8140812, head_top: 0xFFE0609E, head_bottom: 0xFF7A2E62, accent: 0xFFFF8AD8, title: 0xFFFFE6F6, soft: 0xFFF0B6DA, back: 0x2A0C22 },
        Theme { border: 0xFF8F7A2A, top: 0xE8231C0A, bottom: 0xE8100C04, head_top: 0xFFB8922E, head_bottom: 0xFF5E4914, accent: 0xFFFFD24A, title: 0xFFFFF6D0, soft: 0xFFE0CE96, back: 0x221A06 },
    ];
    static THEME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn theme() -> Theme {
        THEMES[THEME.load(std::sync::atomic::Ordering::Relaxed).min(THEMES.len() - 1)]
    }
    const VIS_NAMES: [&str; 3] = ["No", "Arriba", "Abajo"];

    #[derive(Default)]
    pub struct Menu {
        pub widgets: Widgets,
        acts: Vec<(WidgetId, Act, [f32; 4])>,
        search: Option<WidgetId>,
        /// The screen size the widgets were laid out for.
        size: [f32; 2],
        rows: usize,
        scroll: usize,
        /// Slots shown in the list now (after the search), in order.
        shown: Vec<u32>,
        /// Ask the tick to close the screen (it needs the mutable host).
        pub close: bool,
        tab: usize,
        /// Value labels between a pair of - / + buttons: (centre x, y, the "-" button).
        labels: Vec<(f32, f32, Act)>,
        left: [f32; 4],
        right: [f32; 4],
        list_top: f32,
        progress_y: f32,
        tabs_y: f32,
        /// Where the style preview draws.
        preview: [[f32; 4]; 4],
    }

    /// Lower case without accents, for searching.
    pub fn fold(s: &str) -> String {
        s.chars()
            .flat_map(|c| c.to_lowercase())
            .map(|c| match c {
                'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'í' | 'ì' | 'î' | 'ï' => 'i',
                'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
                'ú' | 'ù' | 'û' | 'ü' => 'u',
                'ñ' => 'n',
                'ç' => 'c',
                c => c,
            })
            .collect()
    }

    fn yes(on: bool) -> &'static str {
        if on { "Sí" } else { "No" }
    }

    const PLACE_NAMES: [&str; 3] = ["Arriba", "En medio", "Abajo"];
    const REPEAT_NAMES: [&str; 3] = ["Repetir: lista", "Repetir: una", "Repetir: no"];

    /// A panel: soft shadow, rounded corners, a header with its title and a glowing accent line.
    fn panel(draw: &mut dyn DrawContext, rect: [f32; 4], title: &str) {
        let th = theme();
        let [x, y, w, h] = rect;
        // Shadow.
        draw.fill([x + 3.0, y + 4.0, w, h], 0x50000000);
        draw.fill([x + 1.0, y + 2.0, w + 1.0, h + 1.0], 0x30000000);
        // Border with the corners cut, so it looks rounded.
        draw.fill([x, y - 1.0, w, h + 2.0], th.border);
        draw.fill([x - 1.0, y, w + 2.0, h], th.border);
        draw.fill_gradient([x, y, w, h], th.top, th.bottom);
        draw.fill_gradient([x, y, w, 15.0], th.head_top, th.head_bottom);
        draw.fill([x, y, w, 1.0], 0x30FFFFFF);
        draw.fill([x, y + 15.0, w, 1.0], th.accent);
        draw.fill_gradient([x, y + 16.0, w, 4.0], (th.accent & 0x00FF_FFFF) | 0x40000000, th.accent & 0x00FF_FFFF);
        draw.text(title, [x + 6.0, y + 4.0], th.title, true);
    }

    /// A rounded little chip with a key and what it does.
    fn chip(draw: &mut dyn DrawContext, x: f32, y: f32, key: &str, what: &str) -> f32 {
        let th = theme();
        let kw = draw.text_width(key) + 6.0;
        let ww = draw.text_width(what) + 4.0;
        draw.fill([x + 1.0, y, kw - 2.0, 11.0], th.accent);
        draw.fill([x, y + 1.0, kw, 9.0], th.accent);
        draw.text(key, [x + 3.0, y + 2.0], 0xFF101010, false);
        draw.text(what, [x + kw + 2.0, y + 2.0], th.soft, true);
        kw + ww + 6.0
    }

    impl Menu {
        /// The menu opens: from the top, with the song list fresh.
        pub fn opened(&mut self) {
            self.size = [0.0, 0.0];
            self.scroll = 0;
            self.close = false;
        }

        pub fn scroll_by(&mut self, by: i64) {
            let max = self.shown.len().saturating_sub(self.rows) as i64;
            self.scroll = (self.scroll as i64 + by).clamp(0, max.max(0)) as usize;
        }

        fn add(&mut self, rect: [f32; 4], act: Act) {
            let id = self.widgets.add_button(Button::new(rect, Text::literal("")));
            self.acts.push((id, act, rect));
        }

        /// A "- value +" row.
        fn pair(&mut self, x: f32, y: f32, col: f32, down: Act, up: Act) {
            self.add([x, y, 20.0, ROW_H], down);
            self.add([x + col - 20.0, y, 20.0, ROW_H], up);
            self.labels.push((x + col / 2.0, y + 5.0, down));
        }

        /// Lays the buttons out for a screen of `[w, h]` GUI pixels.
        fn layout(&mut self, [w, h]: [f32; 2]) {
            let search = self.search.and_then(|id| self.widgets.text_field(id)).map(|f| f.value().to_string());
            self.widgets = Widgets::new();
            self.acts.clear();
            self.labels.clear();
            self.size = [w, h];

            let col = 200.0f32.min((w - 40.0) / 2.0);
            let pad = 6.0;
            let cx = w / 2.0;
            let top = (h * 0.06).clamp(16.0, 30.0);
            let panel_h = (h - top - 18.0).min(272.0);
            let lx = cx - col - 8.0;
            let rx = cx + 8.0;
            self.left = [lx - pad, top, col + pad * 2.0, panel_h];
            self.right = [rx - pad, top, col + pad * 2.0, panel_h];

            // Left: search, the song list, and the scroll arrows.
            let field = self.widgets.add_text_field(TextField::new([lx, top + 19.0, col, 16.0], 64));
            if let Some(text) = search {
                if let Some(f) = self.widgets.text_field_mut(field) {
                    f.set_value(&text);
                }
            }
            self.search = Some(field);
            self.list_top = top + 39.0;
            let rows = (((top + panel_h - 26.0 - self.list_top) / ROW_H).floor() as usize).clamp(3, 18);
            self.rows = rows;
            for r in 0..rows {
                self.add([lx, self.list_top + r as f32 * ROW_H, col, ROW_H - 1.0], Act::Row(r));
            }
            let arrows = top + panel_h - 22.0;
            let half = (col - GAP) / 2.0;
            self.add([lx, arrows, half, 16.0], Act::Up);
            self.add([lx + half + GAP, arrows, half, 16.0], Act::Down);

            // Right: what plays (drawn), the controls, the tabs, then the tab's settings.
            self.progress_y = top + 40.0;
            let y = top + 46.0;
            self.add([rx, y, 40.0, ROW_H], Act::Prev);
            self.add([rx + 40.0 + GAP, y, col - 80.0 - GAP * 2.0, ROW_H], Act::Toggle);
            self.add([rx + col - 40.0, y, 40.0, ROW_H], Act::Next);
            self.tabs_y = y + STEP + 4.0;
            let quarter = (col - GAP * (NTABS as f32 - 1.0)) / NTABS as f32;
            for t in 0..NTABS {
                self.add([rx + t as f32 * (quarter + GAP), self.tabs_y, quarter, 16.0], Act::Tab(t));
            }
            let start = self.tabs_y + 22.0;
            let close_y = top + panel_h - 22.0;

            // Música
            let mut y = start;
            self.pair(rx, y, col, Act::VolDown, Act::VolUp);
            y += STEP;
            self.pair(rx, y, col, Act::SyncDown, Act::SyncUp);
            y += STEP;
            self.pair(rx, y, col, Act::SongSyncDown, Act::SongSyncUp);
            y += STEP;
            for (a, b) in [(Act::Shuffle, Act::Repeat), (Act::Widget, Act::Corner), (Act::Crossfade, Act::Banner)] {
                self.add([rx, y, half, ROW_H], a);
                self.add([rx + half + GAP, y, half, ROW_H], b);
                y += STEP;
            }
            self.add([rx, y, col, ROW_H], Act::MuteGame);

            // Letras
            let mut y = start;
            self.add([rx, y, half, ROW_H], Act::Lyrics);
            self.add([rx + half + GAP, y, half, ROW_H], Act::Mode);
            y += STEP;
            self.pair(rx, y, col, Act::SizeDown, Act::SizeUp);
            y += STEP;
            self.pair(rx, y, col, Act::AlphaDown, Act::AlphaUp);
            y += STEP;
            // World: distance, lines and where. Screen: place and context, in the same rows.
            self.pair(rx, y, col, Act::DistDown, Act::DistUp);
            self.add([rx, y, col, ROW_H], Act::Place);
            y += STEP;
            self.pair(rx, y, col, Act::LinesDown, Act::LinesUp);
            self.add([rx, y, col, ROW_H], Act::Context);
            y += STEP;
            self.add([rx, y, half, ROW_H], Act::Layout);
            self.add([rx + half + GAP, y, half, ROW_H], Act::Facing);

            // Estilo
            let mut y = start;
            self.pair(rx, y, col, Act::StyleDown, Act::StyleUp);
            y += STEP;
            self.pair(rx, y, col, Act::ColorDown, Act::ColorUp);
            y += STEP;
            self.pair(rx, y, col, Act::TtfDown, Act::TtfUp);
            y += STEP;
            self.pair(rx, y, col, Act::FontDown, Act::FontUp);
            y += STEP;
            self.add([rx, y, col, ROW_H], Act::Boxed);
            y += STEP + 2.0;
            self.preview[2] = [rx, y, col, (close_y - y - 4.0).max(20.0)];

            // Ritmo
            let mut y = start;
            self.pair(rx, y, col, Act::FxDown, Act::FxUp);
            y += STEP;
            self.pair(rx, y, col, Act::SourceDown, Act::SourceUp);
            y += STEP;
            self.pair(rx, y, col, Act::RhythmDown, Act::RhythmUp);
            y += STEP;
            self.add([rx, y, col, ROW_H], Act::Visualizer);
            y += STEP + 2.0;
            self.preview[3] = [rx, y, col, (close_y - y - 4.0).max(20.0)];

            self.add([rx, close_y, half, ROW_H], Act::Reset);
            self.add([rx + half + GAP, close_y, half, ROW_H], Act::Close);
        }

        /// The songs the search leaves, in their numbered order. It looks in the name, the
        /// identified artist and title and the lyrics' tags, without caring for accents; every
        /// word typed must be there. "?" lists the songs to check, "♪" the instrumentals.
        fn filter(&mut self, all: &[Song]) {
            let query = self.search.and_then(|id| self.widgets.text_field(id)).map(|f| fold(f.value())).unwrap_or_default();
            let words: Vec<&str> = query.split_whitespace().collect();
            self.shown = all
                .iter()
                .filter(|s| {
                    if words.is_empty() {
                        return true;
                    }
                    let mut hay = s.name.clone();
                    if let Some(i) = &s.info {
                        hay.push(' ');
                        hay.push_str(&i.identified);
                    }
                    if let Some(l) = &s.lyrics {
                        for t in [&l.artist, &l.title].into_iter().flatten() {
                            hay.push(' ');
                            hay.push_str(t);
                        }
                    }
                    let hay = fold(&hay);
                    words.iter().all(|w| match *w {
                        "♪" => s.instrumental(),
                        w => hay.contains(w) || s.slot.to_string() == w,
                    })
                })
                .map(|s| s.slot)
                .collect();
            self.scroll_by(0);
        }

        /// A button's label and whether it can be pressed; None hides it.
        fn label(&self, host: &dyn ClientHost, act: Act, player: &Player, all: &[Song]) -> Option<(String, bool)> {
            if act.tab().is_some_and(|t| t != self.tab) {
                return None;
            }
            let b = |k: &str, d: bool| host.setting_bool(k).unwrap_or(d);
            let c = |k: &str, d: u32| host.setting_choice(k).unwrap_or(d) as usize;
            let playing = player.now.as_ref().map(|p| p.song.slot);
            let world = c("lyrics_mode", 0) == 0;
            Some(match act {
                Act::Row(r) => {
                    let slot = *self.shown.get(self.scroll + r)?;
                    let song = all.iter().find(|s| s.slot == slot)?;
                    let mark = match &song.lyrics {
                        Some(l) if l.lines.iter().any(|x| !x.words.is_empty()) => " ♫✦".to_string(),
                        Some(_) => " ♫".to_string(),
                        None if song.instrumental() => " ♪".to_string(),
                        None => String::new(),
                    };
                    let lead = if playing == Some(slot) { "▶ " } else { "" };
                    let mut text = format!("{lead}{}", song.name);
                    if text.chars().count() > 34 {
                        text = text.chars().take(32).collect::<String>() + "…";
                    }
                    (format!("{text}{mark}"), true)
                }
                Act::Up => ("▲".into(), self.scroll > 0),
                Act::Down => ("▼".into(), self.scroll + self.rows < self.shown.len()),
                Act::Prev => ("⏮".into(), !all.is_empty()),
                Act::Next => ("⏭".into(), !all.is_empty()),
                Act::Toggle => ((if playing.is_some() { "■ Detener" } else { "▶ Reproducir" }).into(), !all.is_empty()),
                Act::Tab(t) => {
                    // Icons always; the name too when the tabs are wide enough.
                    let wide = (self.right[2] - 12.0 - GAP * (NTABS as f32 - 1.0)) / NTABS as f32 >= 44.0;
                    (if wide { format!("{} {}", TAB_ICONS[t], TABS[t]) } else if t == self.tab { TABS[t].to_string() } else { TAB_ICONS[t].to_string() }, t != self.tab)
                }
                Act::VolDown | Act::SyncDown | Act::SizeDown | Act::RhythmDown | Act::AlphaDown => ("-".into(), true),
                Act::VolUp | Act::SyncUp | Act::SizeUp | Act::RhythmUp | Act::AlphaUp => ("+".into(), true),
                Act::SongSyncDown => ("-".into(), playing.is_some()),
                Act::SongSyncUp => ("+".into(), playing.is_some()),
                Act::Crossfade => (format!("Transición: {}", yes(b("crossfade", true))), true),
                Act::Layout if world => (format!("Dónde: {}", LAYOUT_NAMES[c("world_layout", 0).min(LAYOUT_NAMES.len() - 1)]), true),
                Act::Facing if world => (format!("Mirando: {}", FACING_NAMES[c("world_facing", 0).min(FACING_NAMES.len() - 1)]), true),
                Act::Layout | Act::Facing => return None,
                Act::FxDown | Act::SourceDown => ("◀".into(), true),
                Act::FxUp | Act::SourceUp => ("▶".into(), true),
                Act::Visualizer => (format!("Barras de música: {}", VIS_NAMES[c("visualizer", 2).min(2)]), true),
                Act::StyleDown | Act::ColorDown | Act::TtfDown => ("◀".into(), true),
                Act::StyleUp | Act::ColorUp | Act::TtfUp => ("▶".into(), true),
                Act::FontDown => ("◀".into(), c("lyrics_ttf", 0) == 0),
                Act::FontUp => ("▶".into(), c("lyrics_ttf", 0) == 0),
                Act::Banner => (format!("Título al empezar: {}", yes(b("banner", true))), true),
                Act::DistDown | Act::LinesDown if world => ("-".into(), true),
                Act::DistUp | Act::LinesUp if world => ("+".into(), true),
                Act::DistDown | Act::LinesDown | Act::DistUp | Act::LinesUp => return None,
                Act::Place if !world => (format!("Posición: {}", PLACE_NAMES[c("lyrics_place", 2).min(2)]), true),
                Act::Context if !world => (format!("Ver línea anterior y siguiente: {}", yes(b("lyrics_context", true))), true),
                Act::Place | Act::Context => return None,
                Act::Mode => ((if world { "En el mundo" } else { "En pantalla" }).into(), true),
                Act::Lyrics => (format!("Letras: {}", yes(b("lyrics", true))), true),
                Act::Shuffle => (format!("Aleatorio: {}", yes(b("shuffle", false))), true),
                Act::Repeat => (REPEAT_NAMES[c("repeat", 0).min(2)].to_string(), true),
                Act::Widget => (format!("Recuadro: {}", yes(b("widget", true))), true),
                Act::Corner => ((if c("widget_corner", 0) == 1 { "Esquina: derecha" } else { "Esquina: izquierda" }).into(), b("widget", true)),
                Act::MuteGame => (format!("Callar la música de Minecraft: {}", yes(b("mute_game_music", true))), true),
                Act::Boxed => (format!("Fondo detrás de la letra: {}", yes(b("lyrics_box", false))), true),
                Act::Reset => (format!("↺ Restablecer {}", TABS[self.tab].to_lowercase()), true),
                Act::Close => ("Listo".into(), true),
            })
        }

        pub fn render(&mut self, host: &dyn ClientHost, draw: &mut dyn DrawContext, mouse: [f32; 2], player: &Player) {
            let size = draw.size();
            if size != self.size {
                self.layout(size);
            }
            let all = songs();
            self.filter(&all);
            for (id, act, rect) in self.acts.clone() {
                let label = self.label(host, act, player, &all);
                if let Some(button) = self.widgets.button_mut(id) {
                    match label {
                        Some((text, active)) => {
                            button.label = Text::literal(text);
                            button.active = active;
                            button.rect = rect;
                        }
                        None => {
                            button.active = false;
                            button.rect = OFF;
                        }
                    }
                }
            }

            let [w, h] = size;
            let th = theme();
            draw.fill_gradient([0.0, 0.0, w, h], argb(th.back, 0.66), 0xC8000000);
            let t = (host.millis() % 100_000) as f32 / 1000.0;
            // Music notes drifting up behind the panels, brighter on the beat.
            let kick = player
                .now
                .as_ref()
                .filter(|p| p.start.is_some() && p.pause_at.is_none())
                .and_then(|p| p.song.energy.as_ref().map(|r| r.beat(p.ms(host.millis())).kick))
                .unwrap_or(0.0);
            for n in 0..14u32 {
                let seed = (n as f32 * 12.9898).sin() * 43758.547;
                let r = seed - seed.floor();
                let speed = 8.0 + r * 14.0;
                let ny = h + 10.0 - ((t * speed + r * 900.0) % (h + 30.0));
                let nx = (r * 997.0 % 1.0) * w + (t * 0.6 + n as f32).sin() * 12.0;
                let note = ["♪", "♫", "♬", "♩"][(n % 4) as usize];
                draw.text(note, [nx, ny], argb(mix(th.accent & 0xFFFFFF, 0xFFFFFF, kick * 0.5), 0.10 + 0.25 * kick + 0.08 * r), false);
            }
            let title = "♪ Show the Lyrics Horizon ♪";
            let tw = draw.text_width(title);
            let ty = self.left[1] - 13.0;
            draw.fill_gradient([w / 2.0 - tw / 2.0 - 16.0, ty - 2.0, tw / 2.0 + 16.0, 12.0], th.accent & 0x00FF_FFFF, (th.accent & 0x00FF_FFFF) | 0x50000000);
            draw.fill_gradient([w / 2.0, ty - 2.0, tw / 2.0 + 16.0, 12.0], (th.accent & 0x00FF_FFFF) | 0x50000000, th.accent & 0x00FF_FFFF);
            draw.text(title, [w / 2.0 - tw / 2.0, ty], argb(mix(hsv(t / 6.0), th.title & 0xFFFFFF, 0.35), 1.0), true);
            // The keys, as little chips under the panels.
            let [_, ly0, _, lh0] = self.left;
            let keys = [("M", "play"), ("N", "siguiente"), ("B", "anterior"), ("K", "letra"), (",", "antes"), (".", "después"), ("J", "menú")];
            let total: f32 = keys.iter().map(|(k, x)| draw.text_width(k) + 6.0 + draw.text_width(x) + 4.0 + 6.0).sum();
            if total < w - 8.0 {
                let mut kx = w / 2.0 - total / 2.0;
                for (k, x) in keys {
                    kx += chip(draw, kx, ly0 + lh0 + 3.0, k, x);
                }
            }
            let with_lyrics = all.iter().filter(|s| s.lyrics.is_some()).count();
            panel(draw, self.left, &format!("Canciones ({})", all.len()));
            panel(draw, self.right, "Sonando ahora");
            let count = format!("{with_lyrics} con letra ♫");
            let [lx, ly, lw, _] = self.left;
            draw.text(&count, [lx + lw - 6.0 - draw.text_width(&count), ly + 3.0], 0xFFC8B8EE, true);

            // Stripes behind the list rows and the playing one highlighted.
            let playing = player.now.as_ref().map(|p| p.song.slot);
            for r in 0..self.rows {
                let Some(slot) = self.shown.get(self.scroll + r) else { break };
                let y = self.list_top + r as f32 * ROW_H - 1.0;
                let colour = if playing == Some(*slot) { (th.accent & 0x00FF_FFFF) | 0x80000000 } else if r % 2 == 0 { 0x18FFFFFF } else { 0 };
                if colour != 0 {
                    draw.fill([lx + 3.0, y, lw - 6.0, ROW_H + 1.0], colour);
                }

            }
            // The tab underline.
            let [rx, ry, rw, _] = self.right;
            let quarter = (rw - 12.0 - GAP * (NTABS as f32 - 1.0)) / NTABS as f32;
            draw.fill([rx + 6.0 + self.tab as f32 * (quarter + GAP), self.tabs_y + 16.0, quarter, 2.0], th.accent);
            draw.fill_gradient([rx + 6.0 + self.tab as f32 * (quarter + GAP), self.tabs_y + 18.0, quarter, 4.0], (th.accent & 0x00FF_FFFF) | 0x60000000, th.accent & 0x00FF_FFFF);

            let _ = self.widgets.draw(draw, host, mouse);
            // How sure each identification is: green, yellow, red, blue for instrumentals.
            for r in 0..self.rows {
                let Some(slot) = self.shown.get(self.scroll + r) else { break };
                let y = self.list_top + r as f32 * ROW_H - 1.0;
                // The one playing: little dancing bars at the end of its row.
                if playing == Some(*slot) && player.now.as_ref().is_some_and(|p| p.start.is_some() && p.pause_at.is_none()) {
                    for b in 0..3 {
                        let v = ((t * (6.0 + b as f32 * 2.1)).sin() * 0.5 + 0.5) * 7.0 + 2.0;
                        draw.fill([lx + lw - 16.0 + b as f32 * 3.0, y + ROW_H - 3.0 - v, 2.0, v], th.title);
                    }
                }
            }

            let centre = lx + lw / 2.0;
            if all.is_empty() {
                for (i, line) in ["No hay canciones todavía.", "Ponlas en PON_TUS_CANCIONES_AQUI,", "abre AGREGAR_CANCIONES.bat", "y pulsa F3+T."].iter().enumerate() {
                    draw.text(line, [centre - draw.text_width(line) / 2.0, self.list_top + 8.0 + i as f32 * 11.0], 0xFFFFFF55, true);
                }
            } else if self.shown.is_empty() {
                let line = "Ninguna canción coincide";
                draw.text(line, [centre - draw.text_width(line) / 2.0, self.list_top + 8.0], 0xFFAAAAAA, true);
            }
            if self.search.and_then(|id| self.widgets.text_field(id)).is_some_and(|f| f.value().is_empty())
                && self.widgets.focused() != self.search
            {
                draw.text("Buscar artista o título…", [lx + 10.0, ly + 23.0], 0xFF777777, false);
            }

            // What plays, with its progress.
            let inner = rw - 12.0;
            let now_ms = host.millis();
            let (mut now, time, progress) = match &player.now {
                Some(p) if p.start.is_none() => (p.song.name.clone(), "Cargando…".to_string(), None),
                Some(p) => {
                    let ms = p.ms(now_ms);
                    match p.song.duration {
                        Some(d) if d > 0 => (p.song.name.clone(), format!("{} / {}", clock(ms), clock(d)), Some((ms as f32 / d as f32).clamp(0.0, 1.0))),
                        _ => (p.song.name.clone(), clock(ms), None),
                    }
                }
                None => ("Nada sonando".to_string(), "Elige una canción de la lista".to_string(), None),
            };
            let time = match player.now.as_ref() {
                Some(p) if p.song.instrumental() => format!("{time}  · instrumental"),
                Some(p) if p.song.lyrics.is_none() => format!("{time}  · sin letra"),
                _ => time,
            };
            // A cover: a square in the song's own colours with a note that beats with the music.
            let cover = 20.0;
            let cx0 = rx + 6.0;
            let cy0 = ry + 19.0;
            let slot_seed = player.now.as_ref().map(|p| p.song.slot as f32).unwrap_or(0.0);
            let h1 = (slot_seed * 0.618).fract();
            draw.fill([cx0 - 1.0, cy0 - 1.0, cover + 2.0, cover + 2.0], th.border);
            draw.fill_gradient([cx0, cy0, cover, cover], argb(hsv(h1), 1.0), argb(mix(hsv(h1 + 0.3), 0x000000, 0.4), 1.0));
            let pulse = 1.0 + 0.25 * kick;
            let ns = 1.4 * pulse;
            let nw = draw.text_width("♫") * ns;
            paint(draw, "♫", cx0 + cover / 2.0 - nw / 2.0, cy0 + cover / 2.0 - 4.0 * ns, ns, 0xFFFFFFFF, true);
            let tx = cx0 + cover + 5.0;
            let inner_t = rx + rw - 40.0 - tx;
            while draw.text_width(&now) > inner_t && now.chars().count() > 4 {
                now.pop();
            }
            draw.text(&now, [tx, ry + 19.0], 0xFFFFFFFF, true);
            // A little live equalizer beside the panel's title, and the tempo.
            if let Some(p) = player.now.as_ref().filter(|p| p.start.is_some() && p.pause_at.is_none()) {
                if let Some(r) = &p.song.energy {
                    let ms = p.ms(now_ms);
                    let i = (ms / 50).max(0) as usize;
                    for b in 0..10usize {
                        let v = r.level.get(i.saturating_sub(b)).copied().unwrap_or(0) as f32 / 9.0;
                        let hgt = 1.0 + v * 8.0;
                        let x = rx + rw - 8.0 - (b as f32 + 1.0) * 4.0;
                        draw.fill([x, ry + 12.0 - hgt, 3.0, hgt], argb(hsv(b as f32 * 0.07 + t / 8.0), 0.9));
                    }
                    let beat = r.beat(ms);
                    if beat.bpm > 0.0 {
                        let bt = format!("♥ {:.0} BPM", beat.bpm);
                        let col = argb(mix(0xBBAADD, 0xFF6688, beat.tempo), 1.0);
                        draw.text(&bt, [rx + rw - 6.0 - draw.text_width(&bt), ry + 29.0], col, true);
                    }
                }
            }
            draw.text(&time, [tx, ry + 29.0], th.soft, true);
            draw.fill([rx + 6.0, self.progress_y, inner, 3.0], 0x40FFFFFF);
            if let Some(f) = progress {
                draw.fill_gradient([rx + 6.0, self.progress_y, inner * f, 3.0], th.accent, (th.title & 0x00FF_FFFF) | 0xFF00_0000);
                draw.fill([rx + 4.0 + inner * f, self.progress_y - 2.0, 4.0, 7.0], 0x60FFFFFF);
                draw.fill([rx + 5.0 + inner * f, self.progress_y - 1.0, 2.0, 5.0], 0xFFFFFFFF);
            }
            // Little equalizer bars while it plays.
            if player.now.as_ref().is_some_and(|p| p.start.is_some() && p.pause_at.is_none()) {
                for i in 0..5 {
                    let v = ((t * (5.0 + i as f32 * 1.7)).sin() * 0.5 + 0.5) * 8.0 + 1.0;
                    draw.fill([rx + rw - 32.0 + i as f32 * 4.0, ry + 37.0 - v, 3.0, v], argb(hsv(t / 5.0 + i as f32 * 0.1), 0.9) | 0xFF00_0000);
                }
            }

            for (x, y, act) in self.labels.clone() {
                if act.tab() != Some(self.tab) {
                    continue;
                }
                let world = host.setting_choice("lyrics_mode").unwrap_or(0) == 0;
                let text = match act {
                    Act::VolDown => format!("Volumen: {}%", host.setting_f64("volume").unwrap_or(100.0).round()),
                    Act::SyncDown => {
                        let o = host.setting_f64("lyrics_offset").unwrap_or(0.0) / 1000.0;
                        if o.abs() < 0.001 { "Sincronía: exacta".to_string() } else if o > 0.0 { format!("Letra antes: {o:.2} s") } else { format!("Letra después: {:.2} s", -o) }
                    }
                    Act::SizeDown => format!("Tamaño de letra: {:.1}", host.setting_f64("lyrics_size").unwrap_or(2.0)),
                    Act::SongSyncDown => {
                        let o = player.now.as_ref().and_then(|p| player.song_offset.get(&p.song.slot)).copied().unwrap_or(0) as f64 / 1000.0;
                        if player.now.is_none() { "Esta canción: -".to_string() } else if o.abs() < 0.001 { "Esta canción: exacta".to_string() } else if o > 0.0 { format!("Esta canción: {o:.2} s antes") } else { format!("Esta canción: {:.2} s después", -o) }
                    }
                    Act::AlphaDown => format!("Opacidad de la letra: {}%", host.setting_f64("lyrics_opacity").unwrap_or(100.0).round()),
                    Act::DistDown if world => format!("Distancia: {:.1} bloques", host.setting_f64("lyrics_distance").unwrap_or(5.0)),
                    Act::LinesDown if world => format!("Líneas a la vez: {}", host.setting_f64("lyrics_lines").unwrap_or(3.0) as i64),
                    Act::StyleDown => format!("Estilo: {}", STYLE_NAMES[(host.setting_choice("lyrics_style").unwrap_or(1) as usize).min(STYLE_NAMES.len() - 1)]),
                    Act::ColorDown => format!("Color: {}", COLOR_NAMES[(host.setting_choice("lyrics_color").unwrap_or(0) as usize).min(COLOR_NAMES.len() - 1)]),
                    Act::FxDown => format!("Efecto: {}", FX_NAMES[(host.setting_choice("rhythm_fx").unwrap_or(14) as usize).min(FX_NAMES.len() - 1)]),
                    Act::SourceDown => format!("Ritmo según: {}", SOURCE_NAMES[(host.setting_choice("rhythm_source").unwrap_or(0) as usize).min(SOURCE_NAMES.len() - 1)]),
                    Act::TtfDown => format!("Fuente: {}", TTF_NAMES[ttf_setting(host)]),
                    Act::FontDown if ttf_setting(host) > 0 => "Letras especiales: solo con Minecraft".to_string(),
                    Act::FontDown => format!("Letras especiales: {}", FONT_NAMES[(host.setting_choice("lyrics_font").unwrap_or(0) as usize).min(FONT_NAMES.len() - 1)]),
                    Act::RhythmDown => {
                        let r = host.setting_f64("rhythm").unwrap_or(60.0);
                        if r < 1.0 { "Ritmo: apagado".to_string() } else { format!("Ritmo: {}%", r.round()) }
                    }
                    _ => continue,
                };
                draw.text(&text, [x - draw.text_width(&text) / 2.0, y], th.title, true);
            }

            // The style tab shows the chosen look, sung over and over.
            if self.tab >= 2 {
                let [px, py, pw, ph] = self.preview[self.tab];
                draw.fill([px, py, pw, ph], 0x60000000);
                draw.fill([px, py, pw, 1.0], 0x40FFFFFF);
                draw.text("Vista previa", [px + 3.0, py + 2.0], th.soft, false);
                // A pretend beat (120 per minute) so the rhythm shows too.
                let ph2 = (host.millis() % 500) as f32 / 500.0;
                let beat = match player.now.as_ref().and_then(|p| p.song.energy.as_ref().map(|r| r.beat(p.ms(host.millis())))) {
                    Some(b) if player.now.as_ref().is_some_and(|p| p.start.is_some()) => b,
                    _ => Beat { kick: (-ph2 / 0.12).exp(), level: 0.6, tempo: (-ph2 / 0.12).exp(), phase: ph2, count: (host.millis() / 500) as f32, bpm: 120.0 },
                };
                let look = Look::from(host).with_beat(beat);
                let cycle = 3800i64;
                let e = (host.millis() % cycle as u64) as i64;
                let ttf_pick = ttf_setting(host);
                let font = if ttf_pick > 0 { 0 } else { (host.setting_choice("lyrics_font").unwrap_or(0) as usize).min(FONT_NAMES.len() - 1) };
                let sample_s = styled("Así se ve tu letra ♪", font);
                let sample = sample_s.as_str();
                set_bold(font == 1);
                prepare_ttf(host, ttf_pick, sample);
                prepare_ttf(host, ttf_pick, " ");
                set_ttf(ttf_pick);
                let scale = (host.setting_f64("lyrics_size").unwrap_or(2.0) as f32).min((pw * 0.9) / super::tw(draw, sample).max(1.0)).min((ph - 12.0) / 11.0).max(0.5);
                let _ = draw.push_clip([px, py, pw, ph]);
                let cy = py + 6.0 + ph / 2.0;
                render_line(draw, sample, px + pw / 2.0, cy, scale, 1.0, &look, 0.0, Some(e - 300), 2200, pw * 0.95, &[]);
                if NO_EXTRAS {
                    sparks(draw, &look, px + pw / 2.0, cy, pw * 0.7, scale.min(1.5));
                }
                let _ = draw.pop_clip();
                set_ttf(0);
                if look.bpm > 0.0 {
                    let t = format!("{:.0} BPM", look.bpm);
                    let on = if look.tempo > 0.5 { 0xFFFFE08A } else { 0xFF9988BB };
                    draw.text(&t, [px + pw - 3.0 - draw.text_width(&t), py + 2.0], on, false);
                }
                set_bold(false);
            }
        }

        pub fn pressed(&mut self, host: &dyn ClientHost, at: [f32; 2], button: i32, modifiers: u32, player: &mut Player) -> bool {
            let outcome = self.widgets.mouse_pressed(host, at, button, modifiers);
            if let Outcome::Pressed(id) = outcome {
                if let Some(&(_, act, _)) = self.acts.iter().find(|(w, _, _)| *w == id) {
                    self.act(host, act, player);
                }
            }
            outcome.handled()
        }

        pub fn key(&mut self, host: &dyn ClientHost, key: &str, modifiers: u32) -> bool {
            let focused = self.focused_search();
            let outcome = self.widgets.key_pressed(host, key, modifiers);
            if outcome.handled() {
                self.scroll = 0;
                return true;
            }
            // While typing a search, letters don't reach the game's keys.
            focused && key != "key.keyboard.escape"
        }

        pub fn typed(&mut self, host: &dyn ClientHost, c: char) -> bool {
            let outcome = self.widgets.char_typed(host, c);
            if outcome.handled() {
                self.scroll = 0;
            }
            outcome.handled()
        }

        fn focused_search(&self) -> bool {
            self.search.is_some() && self.widgets.focused() == self.search
        }

        fn act(&mut self, host: &dyn ClientHost, act: Act, player: &mut Player) {
            let flip = |k: &str, d: bool| {
                let on = host.setting_bool(k).unwrap_or(d);
                host.set_setting(ID, k, if on { "false" } else { "true" });
            };
            let cycle = |k: &str, d: u32, n: u32, by: i32| {
                let v = (host.setting_choice(k).unwrap_or(d) as i32 + by).rem_euclid(n as i32);
                host.set_setting(ID, k, &v.to_string());
            };
            let nudge = |k: &str, d: f64, by: f64, lo: f64, hi: f64| {
                let v = (host.setting_f64(k).unwrap_or(d) + by).clamp(lo, hi);
                host.set_setting(ID, k, &v.to_string());
            };
            match act {
                Act::Row(r) => {
                    if let Some(&slot) = self.shown.get(self.scroll + r) {
                        player.play_slot(host, slot);
                    }
                }
                Act::Up => self.scroll_by(-(self.rows as i64 - 1).max(1)),
                Act::Down => self.scroll_by((self.rows as i64 - 1).max(1)),
                Act::Prev => player.previous(host),
                Act::Toggle => player.toggle(host),
                Act::Next => player.step(host, 1),
                Act::Tab(t) => self.tab = t,
                Act::VolDown | Act::VolUp => {
                    nudge("volume", 100.0, if act == Act::VolUp { 5.0 } else { -5.0 }, 0.0, 100.0);
                    player.apply_volume(host);
                }
                Act::SyncDown | Act::SyncUp => {
                    nudge("lyrics_offset", 0.0, if act == Act::SyncUp { 50.0 } else { -50.0 }, -2000.0, 2000.0)
                }
                Act::SongSyncDown | Act::SongSyncUp => player.nudge_song(if act == Act::SongSyncUp { 50 } else { -50 }),
                Act::AlphaDown | Act::AlphaUp => {
                    nudge("lyrics_opacity", 100.0, if act == Act::AlphaUp { 10.0 } else { -10.0 }, 10.0, 100.0)
                }
                Act::Crossfade => flip("crossfade", true),
                Act::Layout => cycle("world_layout", 0, LAYOUT_NAMES.len() as u32, 1),
                Act::Facing => cycle("world_facing", 0, FACING_NAMES.len() as u32, 1),
                Act::SizeDown | Act::SizeUp => {
                    nudge("lyrics_size", 2.0, if act == Act::SizeUp { 0.5 } else { -0.5 }, 1.0, 4.0)
                }
                Act::DistDown | Act::DistUp => {
                    nudge("lyrics_distance", 5.0, if act == Act::DistUp { 0.5 } else { -0.5 }, 2.0, 12.0)
                }
                Act::LinesDown | Act::LinesUp => {
                    nudge("lyrics_lines", 3.0, if act == Act::LinesUp { 1.0 } else { -1.0 }, 1.0, 6.0)
                }
                Act::RhythmDown | Act::RhythmUp => {
                    nudge("rhythm", 60.0, if act == Act::RhythmUp { 10.0 } else { -10.0 }, 0.0, 100.0)
                }
                Act::StyleDown | Act::StyleUp => cycle("lyrics_style", 1, STYLE_NAMES.len() as u32, if act == Act::StyleUp { 1 } else { -1 }),
                Act::ColorDown | Act::ColorUp => cycle("lyrics_color", 0, COLOR_NAMES.len() as u32, if act == Act::ColorUp { 1 } else { -1 }),
                Act::Mode => cycle("lyrics_mode", 0, 2, 1),
                Act::Place => cycle("lyrics_place", 2, 3, 1),
                Act::Corner => cycle("widget_corner", 0, 2, 1),
                Act::Lyrics => flip("lyrics", true),
                Act::Context => flip("lyrics_context", true),
                Act::Boxed => flip("lyrics_box", false),
                Act::FxDown | Act::FxUp => cycle("rhythm_fx", 14, FX_NAMES.len() as u32, if act == Act::FxUp { 1 } else { -1 }),
                Act::SourceDown | Act::SourceUp => cycle("rhythm_source", 0, SOURCE_NAMES.len() as u32, if act == Act::SourceUp { 1 } else { -1 }),
                Act::TtfDown | Act::TtfUp => cycle("lyrics_ttf", 0, TTF_KEYS.len() as u32, if act == Act::TtfUp { 1 } else { -1 }),
                Act::FontDown | Act::FontUp => cycle("lyrics_font", 0, FONT_NAMES.len() as u32, if act == Act::FontUp { 1 } else { -1 }),
                Act::Banner => flip("banner", true),
                Act::Visualizer => cycle("visualizer", 2, 3, 1),
                Act::Shuffle => {
                    flip("shuffle", false);
                    let all = songs();
                    player.rebuild_order(&all, host.setting_bool("shuffle").unwrap_or(false));
                }
                Act::Repeat => cycle("repeat", 0, 3, 1),
                Act::Widget => flip("widget", true),
                Act::MuteGame => flip("mute_game_music", true),
                Act::Reset => {
                    let defaults: &[(&str, &str)] = match self.tab {
                        0 => &[("volume", "100"), ("lyrics_offset", "0"), ("shuffle", "false"), ("repeat", "0"), ("widget", "true"), ("widget_corner", "0"), ("crossfade", "true"), ("banner", "true"), ("mute_game_music", "true")],
                        1 => &[("lyrics", "true"), ("lyrics_mode", "0"), ("lyrics_size", "2"), ("lyrics_opacity", "100"), ("lyrics_distance", "5"), ("lyrics_lines", "3"), ("lyrics_place", "2"), ("lyrics_context", "true"), ("world_layout", "0"), ("world_facing", "0")],
                        2 => &[("lyrics_style", "1"), ("lyrics_color", "0"), ("lyrics_ttf", "0"), ("lyrics_font", "0"), ("lyrics_box", "false")],
                        _ => &[("rhythm_fx", "14"), ("rhythm_source", "0"), ("rhythm", "60"), ("visualizer", "2")],
                    };
                    for (k, v) in defaults {
                        host.set_setting(ID, k, v);
                    }
                    if self.tab == 0 {
                        player.apply_volume(host);
                        let all = songs();
                        player.rebuild_order(&all, false);
                    }
                }
                Act::Close => self.close = true,
            }
        }
    }
}

// ---------------------------------------------------------------- hooks

fn key(id: &str, default: &str) -> KeyRegistration {
    let mut k = KeyRegistration::new(id, default);
    k.label = format!("musica.key.{id}");
    k.category = "musica.key.category".into();
    k.context(KeyConflictContext::InGame)
}

#[mod_api::horizon_mod]
impl Hooks for Musica {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for s in settings() {
            let _ = r.register_setting(s);
        }
        r.register_key_mapping(key("play", "key.keyboard.m"));
        r.register_key_mapping(key("next", "key.keyboard.n"));
        r.register_key_mapping(key("previous", "key.keyboard.b"));
        r.register_key_mapping(key("lyrics", "key.keyboard.k"));
        r.register_key_mapping(key("menu", "key.keyboard.j"));
        r.register_key_mapping(key("sooner", "key.keyboard.comma"));
        r.register_key_mapping(key("later", "key.keyboard.period"));
        r.register_key_mapping(key("photo", "key.keyboard.o"));
    }

    fn add_client_reload_listeners(&self, _: &dyn ClientHost, event: &mut AddClientReloadListenersEvent<'_>) {
        let add = |l: &mut dyn ClientReloadListeners, name: &str, dir: &str, ext: &str, it: Arc<dyn ClientReloadListener>| {
            if let (Ok(id), Ok(conv)) = (Identifier::parse(&format!("{ID}:{name}")), FileToIdConverter::new(dir, ext)) {
                let _ = l.add_listener(id, conv, it);
            }
        };
        add(&mut *event.listeners, "songs", "sounds/canciones", ".ogg", Arc::new(SongFiles));
        add(&mut *event.listeners, "names", "canciones", ".txt", Arc::new(NameFiles));
        add(&mut *event.listeners, "lyrics", "letras", ".lrc", Arc::new(LyricFiles));
        add(&mut *event.listeners, "rhythm", "ritmo", ".txt", Arc::new(RhythmFiles));
        add(&mut *event.listeners, "info", "info", ".txt", Arc::new(InfoFiles));
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        let play = host.consume_click(ID, "play");
        let next = host.consume_click(ID, "next");
        let prev = host.consume_click(ID, "previous");
        let lyr = host.consume_click(ID, "lyrics");
        let sooner = host.consume_click(ID, "sooner");
        let later = host.consume_click(ID, "later");
        if host.consume_click(ID, "photo") {
            let msg = match host.request_screenshot() {
                Ok(()) => "Foto guardada en la carpeta screenshots",
                Err(mod_api::ScreenshotError::Busy) => "Espera, ya se está sacando una foto",
                Err(mod_api::ScreenshotError::BudgetExhausted) => "Ya sacaste demasiadas fotos en esta sesión",
                Err(_) => "No se pudo sacar la foto",
            };
            lock(&self.player).toast(msg);
        }
        if host.consume_click(ID, "menu") && host.mod_screen().is_none() {
            lock(&self.menu).opened();
            host.mod_screen_open(ID);
        }
        if std::mem::take(&mut lock(&self.menu).close) {
            host.mod_screen_close(ID);
        }
        if lyr {
            let on = host.setting_bool("lyrics").unwrap_or(true);
            host.set_setting(ID, "lyrics", if on { "false" } else { "true" });
        }
        let mut player = lock(&self.player);
        if lyr {
            let on = host.setting_bool("lyrics").unwrap_or(true);
            player.toast(if on { "Letras: visibles" } else { "Letras: ocultas" });
        }
        if sooner {
            player.nudge_song(100);
        }
        if later {
            player.nudge_song(-100);
        }
        if play {
            player.toggle(&*host);
        } else if next {
            player.step(&*host, 1);
        } else if prev {
            player.previous(&*host);
        }
        player.tick(&*host);
    }

    // The camera sways side to side with the beat, if the player wants it.
    fn camera_roll(&self, host: &dyn ClientHost, event: &mut CameraRollEvent) {
        let amp = host.setting_f64("camera_sway").unwrap_or(0.0) as f32;
        if amp <= 0.0 {
            return;
        }
        let player = lock(&self.player);
        let Some(p) = &player.now else { return };
        if p.pause_at.is_some() || p.start.is_none() {
            return;
        }
        let Some(e) = p.song.energy.as_ref() else { return };
        let b = e.beat(p.ms(host.millis()));
        let side = if (b.count as i64) % 2 == 0 { 1.0 } else { -1.0 };
        let swing = if b.bpm > 0.0 { (b.phase * std::f32::consts::PI).sin() * side * (0.4 + 0.6 * b.level) } else { b.kick * side };
        event.roll += amp * swing;
    }

    // The game's own music stays quiet while a song plays.
    fn music(&self, host: &dyn ClientHost, event: &mut MusicEvent<'_>) {
        if host.setting_bool("mute_game_music").unwrap_or(true) && lock(&self.player).now.is_some() {
            *event.music = None;
        }
    }

    fn render_gui_post(&self, host: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
        self.draw(host, &mut *event.draw);
    }

    // ---- the menu (J): its own screen, drawn and clicked through these hooks.

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if event.screen.id == "paused" {
            lock(&self.player).pause_menu_seen = host.millis();
        }
        if !self.my_screen(host, event.screen) {
            return;
        }
        let player = lock(&self.player);
        let mut menu = lock(&self.menu);
        menu.render(host, &mut *event.draw, event.screen.mouse, &player);
    }

    fn screen_mouse_button_pressed_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
        if self.my_screen(host, event.screen) {
            let mut player = lock(&self.player);
            let mut menu = lock(&self.menu);
            if menu.pressed(host, event.screen.mouse, event.button, event.modifiers, &mut player) {
                event.cancelled = true;
            }
        }
        Flow::Continue
    }

    fn screen_mouse_button_released_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseButtonReleasedPreEvent<'_>) -> Flow {
        if self.my_screen(host, event.screen) && lock(&self.menu).widgets.mouse_released(event.button).handled() {
            event.cancelled = true;
        }
        Flow::Continue
    }

    fn screen_mouse_dragged_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseDraggedPreEvent<'_>) -> Flow {
        if self.my_screen(host, event.screen)
            && lock(&self.menu).widgets.mouse_dragged(host, event.screen.mouse, event.button).handled()
        {
            event.cancelled = true;
        }
        Flow::Continue
    }

    fn screen_mouse_scrolled_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if self.my_screen(host, event.screen) {
            lock(&self.menu).scroll_by(-event.scroll[1].signum() as i64 * 2);
            event.cancelled = true;
        }
        Flow::Continue
    }

    fn screen_key_pressed_pre(&self, host: &dyn ClientHost, event: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
        if self.my_screen(host, event.screen) {
            let mut menu = lock(&self.menu);
            if menu.key(host, event.key, event.modifiers) {
                event.cancelled = true;
            }
        }
        Flow::Continue
    }

    fn screen_char_typed_pre(&self, host: &dyn ClientHost, event: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
        if self.my_screen(host, event.screen) && lock(&self.menu).typed(host, event.c) {
            event.cancelled = true;
        }
        Flow::Continue
    }

    // Leaving the world: stop the song.
    fn client_player_logging_out(&self, host: &mut dyn ClientHost, _: &mut ClientPlayerLoggingOutEvent) {
        lock(&self.player).stop(&*host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_reads_tags_stamps_and_offset() {
        let l = parse_lrc("[ti:Hola]\n[ar:Yo]\n[offset:500]\n[00:01.50][00:10.00]uno\n[00:05.2]dos\n");
        assert_eq!(l.title.as_deref(), Some("Hola"));
        assert_eq!(l.artist.as_deref(), Some("Yo"));
        assert_eq!(l.lines.iter().map(|x| x.at).collect::<Vec<_>>(), vec![1000, 4700, 9500]);
        assert_eq!(l.current(0), None);
        assert_eq!(l.current(5000), Some(1));
    }

    #[test]
    fn beat_kicks_on_a_jump() {
        let e = parse_energy(b"1111111119999");
        assert_eq!(e.len(), 13);
        assert!(beat_at(&e, 9 * 50).kick > 0.9);
        assert!(beat_at(&e, 3 * 50).kick < 0.01);
        assert_eq!(beat_at(&e, 99_999).level, 0.0);
    }

    #[test]
    fn tempo_found_at_120() {
        // A beat every 10 frames (500 ms) for 30 s.
        let e: Vec<u8> = (0..600).map(|i| if i % 10 == 3 { 9 } else { 1 }).collect();
        let t = find_tempo(&e);
        assert!(!t.is_empty());
        assert!((t[0].1 - 10.0).abs() < 0.3, "period {}", t[0].1);
        let r = Rhythm { level: e.clone(), bass: Vec::new(), tempo: t };
        assert!((r.beat(2000).bpm - 120.0).abs() < 4.0);
    }

    #[test]
    fn fonts_change_letters() {
        assert_eq!(styled("Hola", 2), "HOLA");
        assert_eq!(styled("ab", 5), "ⓐⓑ");
        assert_eq!(styled("ab", 13), "qɐ");
        assert_eq!(styled("a1", 4), "ａ１");
        assert_eq!(styled("Cz", 11), "ℂ𝕫");
    }

    #[test]
    fn word_times_read() {
        let l = parse_lrc("[00:10.00]<00:10.00>Hola <00:10.50>mundo <00:11.20>bonito");
        assert_eq!(l.lines[0].text, "Hola mundo bonito");
        assert_eq!(l.lines[0].words, vec![0, 500, 1200]);
        let plain = parse_lrc("[00:01.00]sin tiempos");
        assert!(plain.lines[0].words.is_empty());
    }

    #[test]
    fn names_read_numbers_and_skip_comments() {
        let n = parse_names("# hola\n1=Bad Bunny - Monaco\n2 = Otra\nx=no\n");
        assert_eq!(n.get(&1).map(String::as_str), Some("Bad Bunny - Monaco"));
        assert_eq!(n.get(&2).map(String::as_str), Some("Otra"));
        assert_eq!(n.len(), 2);
    }

    #[test]
    fn info_reads_and_judges() {
        let i = parse_info("fuente=lrclib\nconfianza=92\ntipo=linea\nestado=auto\nbloqueada=1\nidentificada=A - B\n");
        assert_eq!(i.source, "lrclib");
        assert_eq!(i.confidence, 92);
        assert!(i.locked && !i.to_check() && !i.instrumental());
        assert!(parse_info("confianza=55\ntipo=linea").to_check());
        assert!(parse_info("tipo=instrumental").instrumental());
    }

    #[test]
    fn lines_together_and_estimated_words() {
        let l = parse_lrc("[00:01.00]uno\n[00:05.00]voz uno\n[00:05.00]voz dos\n[00:09.00]tres");
        assert_eq!(l.group(2), (1, 2));
        assert_eq!(l.group(0), (0, 0));
        assert_eq!(l.next_at(1), Some(9000));
        let t = estimate_words("hola mundo, bonito día", 2000, 0, None);
        assert_eq!(t.len(), 4);
        assert!(t.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(syllables("bonito"), 3);
    }

    #[test]
    fn every_font_writes_something() {
        for f in 0..FONT_NAMES.len() {
            let out = styled("Hola Mundo 123", f);
            assert!(!out.trim().is_empty(), "font {f}");
        }
        assert_eq!(styled("abc", 34), "αβς");
        assert_eq!(styled("ab", 38), "ba");
        assert_eq!(styled("hola", 39), "HoLa");
        assert_eq!(STYLE_KEYS.len(), STYLE_NAMES.len());
        assert_eq!(FX_KEYS.len(), FX_NAMES.len());
        assert_eq!(COLOR_KEYS.len(), COLORS.len() + 1);
    }

    struct Rec;
    impl DrawContext for Rec {
        fn size(&self) -> [f32; 2] { [480.0, 270.0] }
        fn fill(&mut self, _: [f32; 4], _: u32) {}
        fn quad(&mut self, _: [[f32; 2]; 4], _: u32) {}
        fn text(&mut self, _: &str, _: [f32; 2], _: u32, _: bool) {}
        fn gui_paint(&mut self, _: &[GuiDraw]) -> Result<(), TextFormError> { Ok(()) }
        fn text_width(&self, t: &str) -> f32 { 6.0 * t.chars().count() as f32 }
        fn push_clip(&mut self, _: [f32; 4]) -> Result<(), GuiClipRefusal> { Ok(()) }
        fn pop_clip(&mut self) -> Result<(), GuiClipRefusal> { Ok(()) }
    }
    #[test]
    fn every_style_draws_without_panic() {
        let mut d = Rec;
        assert_eq!(tw(&d, "abc"), 18.0);
        for style in 0..STYLE_NAMES.len() {
            for fxi in 0..FX_NAMES.len() {
                for &(bpm, music) in &[(0.0f32, false), (120.0, true)] {
                    let look = Look { style, fx: fxi, rhythm: 0.8, bpm, music, tempo: 0.7, phase: 0.3, count: 5.0, t: 3.3, kick: 0.9, level: 0.5, ..Default::default() };
                    for &e in &[None, Some(-500i64), Some(0), Some(150), Some(900), Some(5000)] {
                        for txt in ["DAKARA BAKIBAKI PAWAACHIISANA YUME WO MAMORINUKUTAME NI", "a", "♪ hola ♪", ""] {
                            render_line(&mut d, txt, 240.0, 130.0, 2.0, 1.0, &look, 0.2, e, 2400, 400.0, &[]);
                            render_line(&mut d, txt, 240.0, 130.0, 0.3, 1.0, &look, 0.2, e, 0, 10.0, &[0, 100]);
                        }
                    }
                }
            }
        }
    }
}
