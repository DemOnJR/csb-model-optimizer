//! GoldSrc studio model (IDST, version 10) reader: just what the optimizer needs.
//!
//! Layouts follow the public HL SDK `studio.h` (`studiohdr_t`, `mstudioseqdesc_t`,
//! `mstudioseqgroup_t`, `mstudioanim_t`, `mstudioanimvalue_t`). Every read is bounds-checked: the
//! input is untrusted, and a broken model is an error, never a panic.

use std::fmt;
use std::ops::Range;

pub const IDST: &[u8; 4] = b"IDST";
pub const IDSQ: &[u8; 4] = b"IDSQ";
pub const VERSION: i32 = 10;

pub const HDR_SIZE: usize = 244;
pub const SEQ_SIZE: usize = 176;
pub const SEQGROUP_SIZE: usize = 104;
pub const BONE_SIZE: usize = 112;
pub const ANIM_SIZE: usize = 12;
pub const EVENT_SIZE: usize = 76;
pub const SEQHDR_SIZE: usize = 76;
const BODYPART_SIZE: usize = 76;
const MODEL_SIZE: usize = 112;
const MESH_SIZE: usize = 20;
const TEXTURE_SIZE: usize = 80;

/// `studiohdr_t` int fields by byte offset.
pub mod hdr {
    pub const LENGTH: usize = 72;
    pub const NUMBONES: usize = 140;
    pub const BONEINDEX: usize = 144;
    pub const NUMBONECONTROLLERS: usize = 148;
    pub const BONECONTROLLERINDEX: usize = 152;
    pub const NUMHITBOXES: usize = 156;
    pub const HITBOXINDEX: usize = 160;
    pub const NUMSEQ: usize = 164;
    pub const SEQINDEX: usize = 168;
    pub const NUMSEQGROUPS: usize = 172;
    pub const SEQGROUPINDEX: usize = 176;
    pub const NUMTEXTURES: usize = 180;
    pub const TEXTUREINDEX: usize = 184;
    pub const TEXTUREDATAINDEX: usize = 188;
    pub const NUMSKINREF: usize = 192;
    pub const NUMSKINFAMILIES: usize = 196;
    pub const SKININDEX: usize = 200;
    pub const NUMBODYPARTS: usize = 204;
    pub const BODYPARTINDEX: usize = 208;
    pub const NUMATTACHMENTS: usize = 212;
    pub const ATTACHMENTINDEX: usize = 216;
    pub const SOUNDINDEX: usize = 224;
    pub const SOUNDGROUPINDEX: usize = 232;
    pub const NUMTRANSITIONS: usize = 236;
    pub const TRANSITIONINDEX: usize = 240;
    /// Header fields that hold file offsets.
    pub const OFFSETS: [usize; 13] = [
        BONEINDEX, BONECONTROLLERINDEX, HITBOXINDEX, SEQINDEX, SEQGROUPINDEX, TEXTUREINDEX,
        TEXTUREDATAINDEX, SKININDEX, BODYPARTINDEX, ATTACHMENTINDEX, SOUNDINDEX, SOUNDGROUPINDEX,
        TRANSITIONINDEX,
    ];
}

/// `mstudioseqdesc_t` fields by byte offset.
pub mod seq {
    pub const NUMEVENTS: usize = 48;
    pub const EVENTINDEX: usize = 52;
    pub const NUMFRAMES: usize = 56;
    pub const NUMPIVOTS: usize = 60;
    pub const PIVOTINDEX: usize = 64;
    pub const AUTOMOVEPOSINDEX: usize = 88;
    pub const AUTOMOVEANGLEINDEX: usize = 92;
    pub const NUMBLENDS: usize = 120;
    pub const ANIMINDEX: usize = 124;
    pub const SEQGROUP: usize = 156;
    /// Fields that hold file offsets (animindex and seqgroup are handled by the writers).
    pub const OFFSETS: [usize; 4] = [EVENTINDEX, PIVOTINDEX, AUTOMOVEPOSINDEX, AUTOMOVEANGLEINDEX];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioError(pub String);

impl fmt::Display for StudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StudioError {}

pub type Result<T> = std::result::Result<T, StudioError>;

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(StudioError(msg.into()))
}

pub fn rd_i32(d: &[u8], off: usize) -> Result<i32> {
    d.get(off..off + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| StudioError(format!("read past the end of the file at {off}")))
}

pub fn rd_u16(d: &[u8], off: usize) -> Result<u16> {
    d.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| StudioError(format!("read past the end of the file at {off}")))
}

pub fn rd_i16(d: &[u8], off: usize) -> Result<i16> {
    Ok(rd_u16(d, off)? as i16)
}

pub fn wr_i32(d: &mut [u8], off: usize, v: i32) {
    d[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn wr_u16(d: &mut [u8], off: usize, v: u16) {
    d[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn cstr(d: &[u8], off: usize, n: usize) -> String {
    let raw = d.get(off..(off + n).min(d.len())).unwrap_or(&[]);
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    raw[..end].iter().map(|&c| c as char).collect()
}

#[derive(Debug, Clone)]
pub struct Seq {
    pub index: usize,
    pub label: String,
    pub numframes: usize,
    pub numblends: usize,
    pub animindex: usize,
    pub seqgroup: i32,
    pub numevents: usize,
}

/// A sequence's animation: `numblends * numbones` entries of 6 channels each.
#[derive(Debug, Clone)]
pub struct SeqAnim {
    pub seq: usize,
    pub numframes: usize,
    pub entries: Vec<[Option<Range<usize>>; 6]>,
    /// The original block (table plus streams) in the file.
    pub block: Range<usize>,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub data: Vec<u8>,
    pub name: String,
    pub numbones: usize,
    pub seqs: Vec<Seq>,
    pub seqgroups: Vec<(String, String)>,
}

impl Model {
    pub fn h(&self, field: usize) -> i32 {
        rd_i32(&self.data, field).unwrap_or(0)
    }

    pub fn hu(&self, field: usize) -> usize {
        self.h(field).max(0) as usize
    }

    pub fn has_internal_textures(&self) -> bool {
        self.h(hdr::TEXTUREINDEX) != 0 && self.h(hdr::NUMTEXTURES) > 0
    }
}

fn check_table(d: &[u8], index: i32, count: i32, size: usize, what: &str) -> Result<()> {
    if !(0..=100_000).contains(&count) {
        return err(format!("{what}: bad count {count}"));
    }
    if count > 0 && (index < 0 || index as usize + count as usize * size > d.len()) {
        return err(format!("{what} table out of range"));
    }
    Ok(())
}

pub fn parse(data: Vec<u8>) -> Result<Model> {
    if data.len() < HDR_SIZE || &data[..4] != IDST {
        return err("not an IDST studio model");
    }
    let version = rd_i32(&data, 4)?;
    if version != VERSION {
        return err(format!("studio version {version}, expected 10"));
    }
    let g = |f| rd_i32(&data, f);
    let numbones = g(hdr::NUMBONES)?;
    check_table(&data, g(hdr::BONEINDEX)?, numbones, BONE_SIZE, "bones")?;
    let numseq = g(hdr::NUMSEQ)?;
    let seqindex = g(hdr::SEQINDEX)?;
    check_table(&data, seqindex, numseq, SEQ_SIZE, "sequences")?;
    let numgroups = g(hdr::NUMSEQGROUPS)?;
    let groupindex = g(hdr::SEQGROUPINDEX)?;
    check_table(&data, groupindex, numgroups, SEQGROUP_SIZE, "sequence groups")?;
    let mut seqs = Vec::new();
    for s in 0..numseq as usize {
        let o = seqindex as usize + s * SEQ_SIZE;
        let numframes = rd_i32(&data, o + seq::NUMFRAMES)?;
        let numblends = rd_i32(&data, o + seq::NUMBLENDS)?;
        let animindex = rd_i32(&data, o + seq::ANIMINDEX)?;
        let numevents = rd_i32(&data, o + seq::NUMEVENTS)?;
        if !(0..=100_000).contains(&numframes) || !(1..=16).contains(&numblends) || animindex < 0 {
            return err(format!("sequence {s}: bad frames/blends/animindex"));
        }
        seqs.push(Seq {
            index: s,
            label: cstr(&data, o, 32),
            numframes: numframes as usize,
            numblends: numblends as usize,
            animindex: animindex as usize,
            seqgroup: rd_i32(&data, o + seq::SEQGROUP)?,
            numevents: numevents.max(0) as usize,
        });
    }
    let mut seqgroups = Vec::new();
    for gi in 0..numgroups as usize {
        let o = groupindex as usize + gi * SEQGROUP_SIZE;
        seqgroups.push((cstr(&data, o, 32), cstr(&data, o + 32, 64)));
    }
    Ok(Model { name: cstr(&data, 8, 64), numbones: numbones as usize, seqs, seqgroups, data })
}

/// Bytes used by one RLE stream covering `numframes` frames.
pub fn stream_span(buf: &[u8], start: usize, numframes: usize) -> Result<usize> {
    let mut p = start;
    let mut covered = 0usize;
    while covered < numframes {
        let (valid, total) = match buf.get(p..p + 2) {
            Some(b) => (b[0] as usize, b[1] as usize),
            None => return err("animation stream runs past the end of the file"),
        };
        if total == 0 {
            return err("animation run with total 0");
        }
        covered += total;
        p += 2 * (valid + 1);
    }
    if p > buf.len() {
        return err("animation stream runs past the end of the file");
    }
    Ok(p - start)
}

/// The channels of a sequence whose table starts at `animindex` in `buf`.
pub fn read_anim(buf: &[u8], animindex: usize, numbones: usize, s: &Seq) -> Result<SeqAnim> {
    let n = s.numblends * numbones;
    let table_end = animindex + n * ANIM_SIZE;
    if table_end > buf.len() {
        return err(format!("sequence {}: animation table out of range", s.label));
    }
    let mut entries = Vec::with_capacity(n);
    let mut end = table_end;
    for i in 0..n {
        let a = animindex + i * ANIM_SIZE;
        let mut chans: [Option<Range<usize>>; 6] = Default::default();
        for (j, c) in chans.iter_mut().enumerate() {
            let off = rd_u16(buf, a + 2 * j)? as usize;
            if off != 0 {
                let st = a + off;
                let span = stream_span(buf, st, s.numframes)?;
                end = end.max(st + span);
                *c = Some(st..st + span);
            }
        }
        entries.push(chans);
    }
    Ok(SeqAnim { seq: s.index, numframes: s.numframes, entries, block: animindex..end })
}

/// Animations of every sequence stored in the model file itself (sequence group 0).
pub fn model_anims(m: &Model) -> Result<Vec<SeqAnim>> {
    m.seqs
        .iter()
        .filter(|s| s.seqgroup == 0)
        .map(|s| read_anim(&m.data, s.animindex, m.numbones, s))
        .collect()
}

/// File positions of every int32 that holds a file offset, except seqdesc animindex and the
/// sequence group data field.
pub fn offset_fields(m: &Model) -> Result<Vec<usize>> {
    let d = &m.data;
    let mut pos: Vec<usize> = hdr::OFFSETS.to_vec();
    let seqindex = m.hu(hdr::SEQINDEX);
    for s in 0..m.seqs.len() {
        let base = seqindex + s * SEQ_SIZE;
        pos.extend(seq::OFFSETS.iter().map(|o| base + o));
    }
    let bpi = m.h(hdr::BODYPARTINDEX);
    let nbp = m.h(hdr::NUMBODYPARTS);
    check_table(d, bpi, nbp, BODYPART_SIZE, "bodyparts")?;
    for b in 0..nbp.max(0) as usize {
        let bp = bpi as usize + b * BODYPART_SIZE;
        pos.push(bp + 72);
        let nummodels = rd_i32(d, bp + 64)?;
        let modelindex = rd_i32(d, bp + 72)?;
        check_table(d, modelindex, nummodels, MODEL_SIZE, "models")?;
        for i in 0..nummodels.max(0) as usize {
            let mo = modelindex as usize + i * MODEL_SIZE;
            pos.extend([76, 84, 88, 96, 100, 108].iter().map(|o| mo + o));
            let nummesh = rd_i32(d, mo + 72)?;
            let meshindex = rd_i32(d, mo + 76)?;
            check_table(d, meshindex, nummesh, MESH_SIZE, "meshes")?;
            for j in 0..nummesh.max(0) as usize {
                let me = meshindex as usize + j * MESH_SIZE;
                pos.push(me + 4);
                pos.push(me + 16);
            }
        }
    }
    if m.h(hdr::TEXTUREINDEX) != 0 {
        let ti = m.h(hdr::TEXTUREINDEX);
        let nt = m.h(hdr::NUMTEXTURES);
        check_table(d, ti, nt, TEXTURE_SIZE, "textures")?;
        pos.extend((0..nt.max(0) as usize).map(|t| ti as usize + t * TEXTURE_SIZE + 76));
    }
    pos.sort_unstable();
    pos.dedup();
    Ok(pos)
}

pub fn merge(mut v: Vec<Range<usize>>) -> Vec<Range<usize>> {
    v.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::new();
    for r in v {
        if let Some(last) = out.last_mut()
            && r.start <= last.end
        {
            last.end = last.end.max(r.end);
            continue;
        }
        out.push(r);
    }
    out
}

/// The contiguous animation region of a model whose sequences are all in group 0. Errors if
/// anything else lives in it or points into it.
pub fn anim_region(m: &Model, anims: &[SeqAnim]) -> Result<Range<usize>> {
    if anims.is_empty() {
        return err("no animation data");
    }
    let merged = merge(anims.iter().map(|a| a.block.clone()).collect());
    for w in merged.windows(2) {
        let gap = &m.data[w[0].end..w[1].start];
        if gap.len() >= 4 || gap.iter().any(|&b| b != 0) {
            return err(format!("non-animation bytes between animation blocks at {}..{}", w[0].end, w[1].start));
        }
    }
    let region = merged[0].start..merged[merged.len() - 1].end;
    for p in offset_fields(m)? {
        let v = rd_i32(&m.data, p)?;
        if v > region.start as i32 && v < region.end as i32 {
            return err(format!("offset at {p} points into the animation region"));
        }
    }
    Ok(region)
}

/// Byte counts by category, for reports.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Breakdown {
    pub file: usize,
    pub animation: usize,
    pub texture: usize,
    pub other: usize,
}

pub fn breakdown(m: &Model) -> Breakdown {
    let file = m.data.len();
    let animation = model_anims(m)
        .map(|a| merge(a.iter().map(|x| x.block.clone()).collect()).iter().map(|r| r.len()).sum())
        .unwrap_or(0);
    let texture = if m.has_internal_textures() && m.hu(hdr::TEXTUREDATAINDEX) <= file {
        file - m.hu(hdr::TEXTUREDATAINDEX)
    } else {
        0
    };
    Breakdown { file, animation, texture, other: file.saturating_sub(animation + texture) }
}

/// Texture pixel+palette blocks (for duplicate checks).
pub fn textures(m: &Model) -> Result<Vec<(String, Range<usize>)>> {
    let mut out = Vec::new();
    if !m.has_internal_textures() {
        return Ok(out);
    }
    let ti = m.hu(hdr::TEXTUREINDEX);
    let nt = m.hu(hdr::NUMTEXTURES);
    check_table(&m.data, ti as i32, nt as i32, TEXTURE_SIZE, "textures")?;
    for t in 0..nt {
        let o = ti + t * TEXTURE_SIZE;
        let w = rd_i32(&m.data, o + 68)?.max(0) as usize;
        let h = rd_i32(&m.data, o + 72)?.max(0) as usize;
        let idx = rd_i32(&m.data, o + 76)?.max(0) as usize;
        let end = idx + w * h + 768;
        if end > m.data.len() {
            return err("texture data out of range");
        }
        out.push((cstr(&m.data, o, 64), idx..end));
    }
    Ok(out)
}
