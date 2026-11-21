//! Animation streams: canonical form, de-duplication and the windowed layout.
//!
//! The engine reads one channel of one bone as `(byte *)panim + panim->offset[j]`, where `panim`
//! is that bone's `mstudioanim_t` and `offset[j]` is an unsigned short. So a stream may be used by
//! any number of entries, from any sequence, as long as it lies 1..=65535 bytes after each of
//! them. studiomdl writes every sequence as its table followed by its own streams; this module
//! packs several sequences into one chunk (their tables, then each distinct stream once) and
//! checks every reference against the 16-bit window.

use std::collections::HashMap;
use std::ops::Range;

use crate::studio::{self, ANIM_SIZE, Result, SeqAnim, StudioError};

pub const MAX_OFFSET: usize = 0xFFFF;

/// The value `StudioCalcBonePosition` / `StudioCalcBoneQuaterion` take for frame `k` of the
/// stream at `start` (the `angle1` / `value[0]` read, including the `valid == 0` quirk where the
/// run header itself is read as the value).
fn values(buf: &[u8], start: usize, numframes: usize) -> Result<Vec<i16>> {
    let mut out = Vec::with_capacity(numframes);
    let mut p = start;
    while out.len() < numframes {
        let b = buf.get(p..p + 2).ok_or_else(|| StudioError("animation stream past the end".into()))?;
        let (valid, total) = (b[0] as usize, b[1] as usize);
        if total == 0 {
            return studio::err("animation run with total 0");
        }
        for k in 0..total {
            if out.len() == numframes {
                break;
            }
            let at = if valid > k { p + 2 + 2 * k } else { p + 2 * valid };
            out.push(studio::rd_i16(buf, at)?);
        }
        p += 2 * (valid + 1);
    }
    Ok(out)
}

/// Decoded values of one channel: None when the channel has no stream (offset 0, which the
/// engine treats exactly like a stream of zeros).
pub fn channel_values(buf: &[u8], stream: &Option<Range<usize>>, numframes: usize) -> Result<Vec<i16>> {
    match stream {
        None => Ok(vec![0; numframes]),
        Some(r) => values(buf, r.start, numframes),
    }
}

/// Canonical bytes of a stream: identical runs except that repeated trailing values of a run are
/// dropped (the engine repeats the last valid value anyway) and the last run claims 255 frames
/// (only frames below `numframes` are ever looked up). Streams that differ only in those
/// details, e.g. a constant channel in sequences of different lengths, become byte-identical.
/// Returns None for an all-zero channel, which needs no stream at all.
pub fn canonical(buf: &[u8], range: &Range<usize>, numframes: usize) -> Result<Option<Vec<u8>>> {
    let orig = values(buf, range.start, numframes)?;
    if orig.iter().all(|&v| v == 0) {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(range.len());
    let mut p = range.start;
    let mut covered = 0usize;
    while covered < numframes {
        let (valid, total) = (buf[p] as usize, buf[p + 1] as usize);
        let vals: Vec<i16> = (0..valid).map(|i| studio::rd_i16(buf, p + 2 + 2 * i)).collect::<Result<_>>()?;
        covered += total;
        let last = covered >= numframes;
        let (mut v, mut t) = (valid, total);
        if valid >= 1 {
            while v > 1 && vals[v - 1] == vals[v - 2] {
                v -= 1;
            }
            if last {
                t = 255;
            }
        }
        out.push(v as u8);
        out.push(t as u8);
        for x in &vals[..v] {
            out.extend_from_slice(&x.to_le_bytes());
        }
        p += 2 * (valid + 1);
    }
    // Belt and braces: keep the original bytes if the rewrite would read differently.
    if values(&out, 0, numframes)? != orig {
        return Ok(Some(buf[range.clone()].to_vec()));
    }
    Ok(Some(out))
}

/// Distinct streams, shared by every job laid out together.
#[derive(Default)]
pub struct Pool {
    pub streams: Vec<Vec<u8>>,
    map: HashMap<Vec<u8>, u32>,
}

impl Pool {
    pub fn intern(&mut self, s: Vec<u8>) -> u32 {
        if let Some(&id) = self.map.get(&s) {
            return id;
        }
        let id = self.streams.len() as u32;
        self.map.insert(s.clone(), id);
        self.streams.push(s);
        id
    }

    pub fn bytes(&self) -> usize {
        self.streams.iter().map(Vec::len).sum()
    }
}

/// One sequence's animation to lay out: its entries as stream ids, plus its original block
/// (table first, relative offsets) to copy verbatim if it cannot be packed.
pub struct Job {
    pub entries: Vec<[Option<u32>; 6]>,
    pub fallback: Vec<u8>,
}

pub fn job(buf: &[u8], a: &SeqAnim, pool: &mut Pool) -> Result<Job> {
    let mut entries = Vec::with_capacity(a.entries.len());
    for e in &a.entries {
        let mut ids = [None; 6];
        for (j, ch) in e.iter().enumerate() {
            if let Some(r) = ch
                && let Some(c) = canonical(buf, r, a.numframes)?
            {
                ids[j] = Some(pool.intern(c));
            }
        }
        entries.push(ids);
    }
    Ok(Job { entries, fallback: buf[a.block.clone()].to_vec() })
}

struct Chunk {
    /// Table position of each job, relative to the chunk start.
    tables: Vec<usize>,
    /// (stream id, position relative to the chunk start) in write order.
    streams: Vec<(u32, usize)>,
    len: usize,
}

fn plan_chunk(jobs: &[&Job], pool: &Pool) -> Option<Chunk> {
    let mut tables = Vec::with_capacity(jobs.len());
    let mut cur = 0usize;
    for j in jobs {
        tables.push(cur);
        cur += j.entries.len() * ANIM_SIZE;
    }
    if cur > MAX_OFFSET {
        return None;
    }
    let mut placed: HashMap<u32, usize> = HashMap::new();
    let mut streams = Vec::new();
    for (j, t) in jobs.iter().zip(&tables) {
        for (i, e) in j.entries.iter().enumerate() {
            let at = t + i * ANIM_SIZE;
            for id in e.iter().flatten() {
                let pos = *placed.entry(*id).or_insert_with(|| {
                    let p = cur;
                    streams.push((*id, p));
                    cur += pool.streams[*id as usize].len();
                    p
                });
                if pos - at > MAX_OFFSET {
                    return None;
                }
            }
        }
    }
    Some(Chunk { tables, streams, len: cur })
}

fn write_chunk(out: &mut Vec<u8>, jobs: &[&Job], c: &Chunk, pool: &Pool) {
    let base = out.len();
    out.resize(base + c.len, 0);
    let pos: HashMap<u32, usize> = c.streams.iter().copied().collect();
    for (j, t) in jobs.iter().zip(&c.tables) {
        for (i, e) in j.entries.iter().enumerate() {
            let at = t + i * ANIM_SIZE;
            for (k, id) in e.iter().enumerate() {
                let off = id.map_or(0, |id| pos[&id] - at);
                studio::wr_u16(out, base + at + 2 * k, off as u16);
            }
        }
    }
    for (id, p) in &c.streams {
        let s = &pool.streams[*id as usize];
        out[base + p..base + p + s.len()].copy_from_slice(s);
    }
}

/// Lays `jobs` out from file position `base`. Returns the bytes and each job's table position
/// (absolute, i.e. `base` + offset). Consecutive jobs share a chunk while every reference fits.
pub fn layout(base: usize, jobs: &[&Job], pool: &Pool) -> (Vec<u8>, Vec<usize>) {
    // A sequence whose channels are exactly those of an earlier one reuses its table: animindex
    // is a full int32, so this works across any distance.
    let mut first: HashMap<&[[Option<u32>; 6]], usize> = HashMap::new();
    let mut dup_of = vec![usize::MAX; jobs.len()];
    let mut uniq: Vec<&Job> = Vec::new();
    let mut uniq_idx = Vec::new();
    for (k, j) in jobs.iter().enumerate() {
        match first.get(j.entries.as_slice()) {
            Some(&f) => dup_of[k] = f,
            None => {
                first.insert(&j.entries, k);
                uniq.push(j);
                uniq_idx.push(k);
            }
        }
    }
    let (out, at_u) = layout_unique(base, &uniq, pool);
    let mut at = vec![0usize; jobs.len()];
    for (u, &k) in uniq_idx.iter().enumerate() {
        at[k] = at_u[u];
    }
    for k in 0..jobs.len() {
        if dup_of[k] != usize::MAX {
            at[k] = at[dup_of[k]];
        }
    }
    (out, at)
}

fn layout_unique(base: usize, jobs: &[&Job], pool: &Pool) -> (Vec<u8>, Vec<usize>) {
    let mut out = Vec::new();
    let mut at = vec![0usize; jobs.len()];
    let mut i = 0;
    while i < jobs.len() {
        while !(base + out.len()).is_multiple_of(4) {
            out.push(0);
        }
        let mut best: Option<(usize, Chunk)> = None;
        let mut j = i + 1;
        while j <= jobs.len() {
            match plan_chunk(&jobs[i..j], pool) {
                Some(c) => best = Some((j, c)),
                None => break,
            }
            j += 1;
        }
        match best {
            Some((j, c)) => {
                let start = out.len();
                write_chunk(&mut out, &jobs[i..j], &c, pool);
                for (k, t) in c.tables.iter().enumerate() {
                    at[i + k] = base + start + t;
                }
                i = j;
            }
            None => {
                at[i] = base + out.len();
                out.extend_from_slice(&jobs[i].fallback);
                i += 1;
            }
        }
    }
    (out, at)
}
