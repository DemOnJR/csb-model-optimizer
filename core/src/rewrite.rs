//! Rewriting models: in-place repack (same file, smaller animation region) and the split into a
//! slim model plus a shared sequence-group file.

use std::ops::Range;

use sha1::{Digest, Sha1};

use crate::layout::{self, Job, Pool};
use crate::studio::{self, Model, Result, hdr, seq, IDSQ, SEQGROUP_SIZE, SEQHDR_SIZE, SEQ_SIZE, VERSION};

pub const SHARED_DIR: &str = "models/player/_anims";

/// The CS game DLL forces these through `force_model_specifybounds` with mp_consistency 1, and
/// the client engine then reads animation at `file + animindex` without looking at the sequence
/// group. Every client already has them, so they are never touched.
pub const CONSISTENCY_FORCED: [&str; 12] = [
    "models/player.mdl",
    "models/player/arctic/arctic.mdl",
    "models/player/gign/gign.mdl",
    "models/player/gsg9/gsg9.mdl",
    "models/player/guerilla/guerilla.mdl",
    "models/player/leet/leet.mdl",
    "models/player/sas/sas.mdl",
    "models/player/terror/terror.mdl",
    "models/player/urban/urban.mdl",
    "models/player/vip/vip.mdl",
    "models/player/spetsnaz/spetsnaz.mdl",
    "models/player/militia/militia.mdl",
];

pub fn is_stock(rel: &str) -> bool {
    CONSISTENCY_FORCED.iter().any(|s| s.eq_ignore_ascii_case(rel))
}

/// None when the model's animation can be rewritten, else why not.
pub fn rewritable(m: &Model) -> Option<&'static str> {
    if m.seqs.is_empty() {
        return Some("no sequences (texture or static model)");
    }
    if m.seqgroups.len() != 1 || m.seqs.iter().any(|s| s.seqgroup != 0) {
        return Some("already uses external sequence groups");
    }
    let gi = m.hu(hdr::SEQGROUPINDEX);
    if studio::rd_i32(&m.data, gi + 100).unwrap_or(1) != 0 {
        return Some("sequence group 0 has a data offset");
    }
    None
}

/// The animation of a model, prepared for layout.
pub struct Prepared {
    pub region: Range<usize>,
    pub jobs: Vec<Job>,
    /// Original animation bytes (the memory a client spends on this model's animation today).
    pub anim_bytes: usize,
}

pub fn prepare(m: &Model, pool: &mut Pool) -> Result<Prepared> {
    let anims = studio::model_anims(m)?;
    let region = studio::anim_region(m, &anims)?;
    let jobs = anims.iter().map(|a| layout::job(&m.data, a, pool)).collect::<Result<Vec<_>>>()?;
    Ok(Prepared { anim_bytes: region.len(), region, jobs })
}

/// Rewrites every offset field: values at or past `end` move by `-cut`, and (for the split)
/// values at or past `insert_at` move by `+insert`.
fn relocate(m: &Model, out: &mut [u8], end: usize, cut: usize, insert_at: usize, insert: usize) -> Result<()> {
    for p in studio::offset_fields(m)? {
        let v = studio::rd_i32(&m.data, p)?;
        if v <= 0 {
            continue;
        }
        let v = v as usize;
        let mut n = if v >= end { v - cut } else { v };
        if v >= insert_at {
            n += insert;
        }
        studio::wr_i32(out, p, n as i32);
    }
    Ok(())
}

/// Same model with its animation region packed (in-model stream de-duplication). Every other
/// byte keeps its meaning; texture data stays 4-byte aligned. None when it would not shrink.
pub fn repack(m: &Model) -> Result<Option<Vec<u8>>> {
    let mut pool = Pool::default();
    let p = prepare(m, &mut pool)?;
    let refs: Vec<&Job> = p.jobs.iter().collect();
    let (mut region, at) = layout::layout(p.region.start, &refs, &pool);
    while (p.region.len() - region.len().min(p.region.len())) % 4 != 0 {
        region.push(0);
    }
    if region.len() >= p.region.len() {
        return Ok(None);
    }
    let cut = p.region.len() - region.len();
    let mut d = m.data.clone();
    relocate(m, &mut d, p.region.end, cut, usize::MAX, 0)?;
    let seqindex = m.hu(hdr::SEQINDEX);
    for (s, a) in m.seqs.iter().zip(&at) {
        studio::wr_i32(&mut d, seqindex + s.index * SEQ_SIZE + seq::ANIMINDEX, *a as i32);
    }
    let mut out = Vec::with_capacity(d.len() - cut);
    out.extend_from_slice(&d[..p.region.start]);
    out.extend_from_slice(&region);
    out.extend_from_slice(&d[p.region.end..]);
    let len = out.len() as i32;
    studio::wr_i32(&mut out, hdr::LENGTH, len);
    Ok(Some(out))
}

/// A shared sequence-group file: the members' sequences laid out together after the IDSQ header.
pub struct Shared {
    pub name: String,
    pub data: Vec<u8>,
    /// Per member, the animindex of each of its sequences inside `data`.
    pub animindex: Vec<Vec<usize>>,
}

/// Builds the shared file for the members' prepared jobs (all interned in `pool`). Jobs are
/// ordered by sequence index, then member, so the same move of different models shares a chunk.
pub fn build_shared(members: &[&[Job]], pool: &Pool) -> Shared {
    let maxseq = members.iter().map(|j| j.len()).max().unwrap_or(0);
    let mut order = Vec::new();
    let mut jobs: Vec<&Job> = Vec::new();
    for s in 0..maxseq {
        for (mi, js) in members.iter().enumerate() {
            if let Some(j) = js.get(s) {
                order.push((mi, s));
                jobs.push(j);
            }
        }
    }
    let (body, at) = layout::layout(SEQHDR_SIZE, &jobs, pool);
    let mut animindex: Vec<Vec<usize>> = members.iter().map(|js| vec![0; js.len()]).collect();
    for ((mi, s), a) in order.iter().zip(at) {
        animindex[*mi][*s] = a;
    }
    let mut h = Sha1::new();
    h.update(&body);
    let name = format!("{SHARED_DIR}/{}.mdl", &hex::encode(h.finalize())[..16]);
    let mut data = vec![0u8; SEQHDR_SIZE];
    data.extend_from_slice(&body);
    data[..4].copy_from_slice(IDSQ);
    studio::wr_i32(&mut data, 4, VERSION);
    data[8..8 + name.len()].copy_from_slice(name.as_bytes());
    let len = data.len() as i32;
    studio::wr_i32(&mut data, 72, len);
    Shared { name, data, animindex }
}

/// The slim model: everything but the animation, whose sequences point into group 1 (`shared`).
pub fn build_slim(m: &Model, shared_name: &str, animindex: &[usize]) -> Result<Vec<u8>> {
    if shared_name.len() > 63 {
        return studio::err("shared file name too long");
    }
    let region = studio::anim_region(m, &studio::model_anims(m)?)?;
    let (a, b) = (region.start, region.end);
    let mut cut = b - a;
    cut -= cut % 4; // keep everything after the region 4-byte aligned
    let sg_end = m.hu(hdr::SEQGROUPINDEX) + SEQGROUP_SIZE;
    if sg_end <= b {
        return studio::err("sequence group table sits before the animation data");
    }
    let mut d = m.data.clone();
    relocate(m, &mut d, b, cut, sg_end, SEQGROUP_SIZE)?;
    let seqindex = m.hu(hdr::SEQINDEX);
    for (s, ai) in m.seqs.iter().zip(animindex) {
        let base = seqindex + s.index * SEQ_SIZE;
        studio::wr_i32(&mut d, base + seq::ANIMINDEX, *ai as i32);
        studio::wr_i32(&mut d, base + seq::SEQGROUP, 1);
    }
    studio::wr_i32(&mut d, hdr::NUMSEQGROUPS, 2);
    let mut group = vec![0u8; SEQGROUP_SIZE];
    group[..6].copy_from_slice(b"shared");
    group[32..32 + shared_name.len()].copy_from_slice(shared_name.as_bytes());
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&d[..a]);
    out.extend(std::iter::repeat_n(0u8, (b - a) - cut));
    out.extend_from_slice(&d[b..sg_end]);
    out.extend_from_slice(&group);
    out.extend_from_slice(&d[sg_end..]);
    let len = out.len() as i32;
    studio::wr_i32(&mut out, hdr::LENGTH, len);
    Ok(out)
}
