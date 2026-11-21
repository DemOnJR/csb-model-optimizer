//! Checks a rewritten model against its original the way the engine would read both: same
//! header values, bones and sequence descriptions, every offset reaching the same bytes, the
//! same texture data, and for every sequence, bone, channel and frame the same animation value
//! (reached through the sequence group, if the model was split).

use crate::layout::channel_values;
use crate::studio::{self, Model, hdr, seq, SEQ_SIZE};

fn masked(m: &Model, fields: &[usize]) -> Vec<u8> {
    let mut out = m.data.clone();
    let mut pos: Vec<usize> = fields.to_vec();
    pos.extend([hdr::LENGTH, hdr::NUMSEQGROUPS]);
    let si = m.hu(hdr::SEQINDEX);
    for s in 0..m.seqs.len() {
        pos.extend([si + s * SEQ_SIZE + seq::ANIMINDEX, si + s * SEQ_SIZE + seq::SEQGROUP]);
    }
    for p in pos {
        if p + 4 <= out.len() {
            out[p..p + 4].fill(0);
        }
    }
    out
}

/// Problems found (empty = identical behaviour). `group` returns the bytes of a sequence-group
/// file by the name stored in the model.
pub fn verify(orig: &Model, new: &Model, group: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<String> {
    let mut problems = Vec::new();
    if let Err(e) = check(orig, new, group, &mut problems) {
        problems.push(e.to_string());
    }
    problems
}

fn check(orig: &Model, new: &Model, group: &dyn Fn(&str) -> Option<Vec<u8>>, out: &mut Vec<String>) -> studio::Result<()> {
    if new.numbones != orig.numbones || new.seqs.len() != orig.seqs.len() {
        out.push("bone or sequence count differs".into());
        return Ok(());
    }
    if new.h(hdr::LENGTH) as usize != new.data.len() {
        out.push("header length does not match the file".into());
    }
    // header: everything but offsets, length and the group count
    let mut ho = orig.data[..studio::HDR_SIZE].to_vec();
    let mut hn = new.data[..studio::HDR_SIZE].to_vec();
    for f in hdr::OFFSETS.iter().chain([hdr::LENGTH, hdr::NUMSEQGROUPS].iter()) {
        ho[*f..f + 4].fill(0);
        hn[*f..f + 4].fill(0);
    }
    if ho != hn {
        out.push("header values changed".into());
    }
    let bone = |m: &Model| {
        let i = m.hu(hdr::BONEINDEX);
        m.data[i..i + m.numbones * studio::BONE_SIZE].to_vec()
    };
    if bone(orig) != bone(new) {
        out.push("bones changed".into());
    }
    // every offset field reaches the same bytes
    let (of, nf) = (studio::offset_fields(orig)?, studio::offset_fields(new)?);
    if of.len() != nf.len() {
        out.push("offset table shape differs".into());
        return Ok(());
    }
    let anims = studio::model_anims(orig)?;
    let region = studio::anim_region(orig, &anims)?;
    let (om, nm) = (masked(orig, &of), masked(new, &nf));
    for (po, pn) in of.iter().zip(&nf) {
        let (vo, vn) = (studio::rd_i32(&orig.data, *po)?, studio::rd_i32(&new.data, *pn)?);
        if vo <= 0 {
            if vo != vn {
                out.push(format!("offset at {po} changed from {vo} to {vn}"));
            }
            continue;
        }
        let (vo, vn) = (vo as usize, vn.max(0) as usize);
        if region.contains(&vo) {
            continue;
        }
        let mut n = 64;
        if vo < region.start {
            n = n.min(region.start - vo);
        }
        let n = n.min(om.len() - vo.min(om.len())).min(nm.len() - vn.min(nm.len()));
        if om.get(vo..vo + n) != nm.get(vn..vn + n) {
            out.push(format!("offset at {po} reaches different bytes"));
            break;
        }
    }
    // sequence descriptions, then the animation itself, value by value
    let (si_o, si_n) = (orig.hu(hdr::SEQINDEX), new.hu(hdr::SEQINDEX));
    let keep = |raw: &[u8]| -> Vec<u8> {
        let mut r = raw.to_vec();
        for f in seq::OFFSETS.iter().chain([seq::ANIMINDEX, seq::SEQGROUP].iter()) {
            r[*f..f + 4].fill(0);
        }
        r
    };
    for (so, sn) in orig.seqs.iter().zip(&new.seqs) {
        let ro = &orig.data[si_o + so.index * SEQ_SIZE..si_o + (so.index + 1) * SEQ_SIZE];
        let rn = &new.data[si_n + sn.index * SEQ_SIZE..si_n + (sn.index + 1) * SEQ_SIZE];
        if keep(ro) != keep(rn) {
            out.push(format!("sequence {} description changed", so.label));
        }
        let ext;
        let buf: &[u8] = if sn.seqgroup == 0 {
            &new.data
        } else {
            let name = match new.seqgroups.get(sn.seqgroup as usize) {
                Some((_, n)) => n,
                None => {
                    out.push(format!("sequence {} points at a missing group", sn.label));
                    continue;
                }
            };
            ext = match group(name) {
                Some(b) => b,
                None => {
                    out.push(format!("shared file {name} not found"));
                    return Ok(());
                }
            };
            &ext
        };
        let ao = studio::read_anim(&orig.data, so.animindex, orig.numbones, so)?;
        let an = match studio::read_anim(buf, sn.animindex, new.numbones, sn) {
            Ok(a) => a,
            Err(e) => {
                out.push(format!("animation of {}: {e}", sn.label));
                continue;
            }
        };
        'chan: for (eo, en) in ao.entries.iter().zip(&an.entries) {
            for (co, cn) in eo.iter().zip(en) {
                if channel_values(&orig.data, co, so.numframes)? != channel_values(buf, cn, sn.numframes)? {
                    out.push(format!("animation of {} differs", so.label));
                    break 'chan;
                }
            }
        }
    }
    // texture data
    if orig.has_internal_textures() {
        let (to, tn) = (orig.hu(hdr::TEXTUREDATAINDEX), new.hu(hdr::TEXTUREDATAINDEX));
        if orig.data.get(to..) != new.data.get(tn..) {
            out.push("texture data differs".into());
        }
    }
    Ok(())
}
