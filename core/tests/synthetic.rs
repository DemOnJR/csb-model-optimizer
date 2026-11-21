//! Synthetic GoldSrc v10 models (bones, RLE animation, one mesh, one texture), rewritten and
//! checked: repack, split into slim + shared, verification catching tampering, the 16-bit
//! offset window, and a whole-folder run. No game files needed.

use csb_mdl::gz::Gzip;
use csb_mdl::layout::{self, Pool};
use csb_mdl::process::{self, Options};
use csb_mdl::rewrite;
use csb_mdl::studio::{self, hdr, seq, ANIM_SIZE, SEQ_SIZE};
use csb_mdl::verify::verify;

fn put_i32(b: &mut [u8], at: usize, v: i32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Channel 0 varies with `seed`, channel 1 is an explicit stream of zeros, channel 3 is constant
/// (one run), the rest have no stream.
fn anim_block(numbones: usize, numframes: usize, seed: i32) -> Vec<u8> {
    let mut table = vec![0u8; numbones * ANIM_SIZE];
    let mut streams = Vec::new();
    for b in 0..numbones {
        let entry = b * ANIM_SIZE;
        let rel = |streams: &Vec<u8>| (table.len() - entry + streams.len()) as u16;
        let o0 = rel(&streams);
        streams.extend([numframes as u8, numframes as u8]);
        for f in 0..numframes {
            let v = ((seed * 7 + b as i32 * 3 + f as i32) % 500 - 250) as i16;
            streams.extend(v.to_le_bytes());
        }
        let o1 = rel(&streams);
        streams.extend([1u8, numframes as u8, 0, 0]);
        let o3 = rel(&streams);
        streams.extend([1u8, numframes as u8]);
        streams.extend((100 + b as i16).to_le_bytes());
        for (j, o) in [(0, o0), (1, o1), (3, o3)] {
            table[entry + 2 * j..entry + 2 * j + 2].copy_from_slice(&o.to_le_bytes());
        }
    }
    table.extend(streams);
    table
}

/// `seqs`: (numframes, seed); `groups`: sequence group count.
fn make_model(name: &str, numbones: usize, seqs: &[(usize, i32)], tex_seed: u8, groups: usize) -> Vec<u8> {
    let mut out = vec![0u8; studio::HDR_SIZE];
    let align = |out: &mut Vec<u8>| {
        while !out.len().is_multiple_of(4) {
            out.push(0)
        }
    };
    align(&mut out);
    let boneindex = out.len();
    for i in 0..numbones {
        let mut b = vec![0u8; studio::BONE_SIZE];
        b[..5].copy_from_slice(b"bone0");
        b[4] = b'0' + (i % 10) as u8;
        put_i32(&mut b, 32, i as i32 - 1);
        for k in 0..6 {
            put_i32(&mut b, 36 + 4 * k, -1);
            b[60 + 4 * k..64 + 4 * k].copy_from_slice(&(0.5 * i as f32).to_le_bytes());
            b[84 + 4 * k..88 + 4 * k].copy_from_slice(&0.01f32.to_le_bytes());
        }
        out.extend(b);
    }
    let hitboxindex = out.len();
    let mut animidx = Vec::new();
    for (frames, seed) in seqs {
        align(&mut out);
        animidx.push(out.len());
        out.extend(anim_block(numbones, *frames, *seed));
    }
    align(&mut out);
    let seqindex = out.len();
    for (i, ((frames, _), ai)) in seqs.iter().zip(&animidx).enumerate() {
        let mut d = vec![0u8; SEQ_SIZE];
        let label = format!("seq{i}");
        d[..label.len()].copy_from_slice(label.as_bytes());
        d[32..36].copy_from_slice(&30.0f32.to_le_bytes());
        put_i32(&mut d, seq::NUMFRAMES, *frames as i32);
        put_i32(&mut d, seq::NUMBLENDS, 1);
        put_i32(&mut d, seq::ANIMINDEX, *ai as i32);
        out.extend(d);
    }
    let seqgroupindex = out.len();
    for _ in 0..groups {
        let mut g = vec![0u8; 104];
        g[..7].copy_from_slice(b"default");
        out.extend(g);
    }
    let bodypartindex = out.len();
    out.extend(vec![0u8; 76]);
    let modelindex = out.len();
    out.extend(vec![0u8; 112]);
    let meshindex = out.len();
    out.extend(vec![0u8; 20]);
    let vertinfo = out.len();
    out.extend([0u8; 4]);
    let verts = out.len();
    out.extend((0..12).flat_map(|i| (i as f32).to_le_bytes()));
    let norminfo = out.len();
    out.extend([0u8; 4]);
    let norms = out.len();
    out.extend((12..24).flat_map(|i| (i as f32).to_le_bytes()));
    let tris = out.len();
    out.extend([3u8, 0].iter().chain([0u8; 16].iter()));
    align(&mut out);
    let textureindex = out.len();
    out.extend(vec![0u8; 80]);
    let skinindex = out.len();
    out.extend([0u8; 4]);
    let texturedataindex = out.len();
    out.extend((0..16 + 768).map(|i| (tex_seed as usize + i) as u8));
    // nested records
    out[bodypartindex..bodypartindex + 4].copy_from_slice(b"body");
    put_i32(&mut out, bodypartindex + 64, 1);
    put_i32(&mut out, bodypartindex + 68, 1);
    put_i32(&mut out, bodypartindex + 72, modelindex as i32);
    out[modelindex..modelindex + 4].copy_from_slice(b"mesh");
    put_i32(&mut out, modelindex + 72, 1);
    put_i32(&mut out, modelindex + 76, meshindex as i32);
    put_i32(&mut out, modelindex + 80, 1);
    put_i32(&mut out, modelindex + 84, vertinfo as i32);
    put_i32(&mut out, modelindex + 88, verts as i32);
    put_i32(&mut out, modelindex + 92, 1);
    put_i32(&mut out, modelindex + 96, norminfo as i32);
    put_i32(&mut out, modelindex + 100, norms as i32);
    put_i32(&mut out, meshindex, 1);
    put_i32(&mut out, meshindex + 4, tris as i32);
    put_i32(&mut out, meshindex + 12, 1);
    put_i32(&mut out, meshindex + 16, norms as i32);
    out[textureindex..textureindex + 8].copy_from_slice(b"skin.bmp");
    put_i32(&mut out, textureindex + 68, 4);
    put_i32(&mut out, textureindex + 72, 4);
    put_i32(&mut out, textureindex + 76, texturedataindex as i32);
    out[..4].copy_from_slice(studio::IDST);
    put_i32(&mut out, 4, 10);
    out[8..8 + name.len()].copy_from_slice(name.as_bytes());
    let len = out.len() as i32;
    put_i32(&mut out, hdr::LENGTH, len);
    for (f, v) in [
        (hdr::NUMBONES, numbones),
        (hdr::NUMSEQ, seqs.len()),
        (hdr::NUMSEQGROUPS, groups),
        (hdr::NUMTEXTURES, 1),
        (hdr::NUMSKINREF, 1),
        (hdr::NUMSKINFAMILIES, 1),
        (hdr::NUMBODYPARTS, 1),
        (hdr::BONEINDEX, boneindex),
        (hdr::BONECONTROLLERINDEX, hitboxindex),
        (hdr::HITBOXINDEX, hitboxindex),
        (hdr::ATTACHMENTINDEX, hitboxindex),
        (hdr::SEQINDEX, seqindex),
        (hdr::SEQGROUPINDEX, seqgroupindex),
        (hdr::TEXTUREINDEX, textureindex),
        (hdr::TEXTUREDATAINDEX, texturedataindex),
        (hdr::SKININDEX, skinindex),
        (hdr::BODYPARTINDEX, bodypartindex),
        (hdr::TRANSITIONINDEX, bodypartindex),
    ] {
        put_i32(&mut out, f, v as i32);
    }
    out
}

const SEQS: &[(usize, i32)] = &[(40, 1), (30, 2), (40, 1), (20, 3)];

#[test]
fn synthetic_model_parses() {
    let m = studio::parse(make_model("a", 8, SEQS, 1, 1)).unwrap();
    assert_eq!(m.seqs.len(), 4);
    assert!(rewrite::rewritable(&m).is_none());
    let anims = studio::model_anims(&m).unwrap();
    assert!(studio::anim_region(&m, &anims).is_ok());
}

#[test]
fn repack_shrinks_and_verifies() {
    let m = studio::parse(make_model("a", 8, SEQS, 1, 1)).unwrap();
    let d = rewrite::repack(&m).unwrap().expect("smaller");
    assert!(d.len() < m.data.len());
    assert_eq!((m.data.len() - d.len()) % 4, 0, "texture data stays aligned");
    let n = studio::parse(d).unwrap();
    assert_eq!(verify(&m, &n, &|_| None), Vec::<String>::new());
    // sequences 0 and 2 are identical: one table
    assert_eq!(n.seqs[0].animindex, n.seqs[2].animindex);
    assert_ne!(n.seqs[0].animindex, n.seqs[1].animindex);
    // the zero channel lost its stream, the constant one is shared between sequences
    let a0 = studio::read_anim(&n.data, n.seqs[0].animindex, 8, &n.seqs[0]).unwrap();
    let a1 = studio::read_anim(&n.data, n.seqs[1].animindex, 8, &n.seqs[1]).unwrap();
    assert!(a0.entries[0][1].is_none());
    assert_eq!(n.data[a0.entries[0][3].clone().unwrap()], n.data[a1.entries[0][3].clone().unwrap()]);
    // repacking again changes nothing
    assert!(rewrite::repack(&n).unwrap().is_none());
}

#[test]
fn verify_catches_changes() {
    let m = studio::parse(make_model("a", 8, SEQS, 1, 1)).unwrap();
    let d = rewrite::repack(&m).unwrap().unwrap();
    let n = studio::parse(d.clone()).unwrap();
    // one animation value
    let a = studio::read_anim(&n.data, n.seqs[1].animindex, 8, &n.seqs[1]).unwrap();
    let mut t = d.clone();
    t[a.entries[2][0].clone().unwrap().start + 4] ^= 1;
    let p = verify(&m, &studio::parse(t).unwrap(), &|_| None);
    assert!(p.iter().any(|x| x.contains("animation of seq1")), "{p:?}");
    // one texture byte
    let mut t = d.clone();
    *t.last_mut().unwrap() ^= 0xFF;
    assert!(verify(&m, &studio::parse(t).unwrap(), &|_| None).iter().any(|x| x.contains("texture")));
    // a bone
    let mut t = d;
    let bi = n.hu(hdr::BONEINDEX) + studio::BONE_SIZE + 70;
    t[bi] ^= 1;
    assert!(verify(&m, &studio::parse(t).unwrap(), &|_| None).iter().any(|x| x.contains("bones")));
}

#[test]
fn split_into_slim_and_shared() {
    let a = studio::parse(make_model("a", 8, SEQS, 1, 1)).unwrap();
    let b = studio::parse(make_model("b", 8, &[(40, 1), (30, 2), (25, 9), (20, 3)], 2, 1)).unwrap();
    let mut pool = Pool::default();
    let pa = rewrite::prepare(&a, &mut pool).unwrap();
    let pb = rewrite::prepare(&b, &mut pool).unwrap();
    let s = rewrite::build_shared(&[&pa.jobs, &pb.jobs], &pool);
    assert!(s.name.starts_with("models/player/_anims/") && s.name.ends_with(".mdl"));
    assert_eq!(&s.data[..4], studio::IDSQ);
    assert_eq!(studio::rd_i32(&s.data, 72).unwrap() as usize, s.data.len());
    assert_eq!(studio::cstr(&s.data, 8, 64), s.name);
    // seq0 of a and seq0 of b are the same animation: one table
    assert_eq!(s.animindex[0][0], s.animindex[1][0]);
    for (m, ai) in [(&a, &s.animindex[0]), (&b, &s.animindex[1])] {
        let slim = studio::parse(rewrite::build_slim(m, &s.name, ai).unwrap()).unwrap();
        assert_eq!(slim.seqgroups.len(), 2);
        assert_eq!(slim.seqgroups[1].1, s.name);
        assert!(slim.seqs.iter().all(|q| q.seqgroup == 1));
        assert!(slim.data.len() < m.data.len());
        assert_eq!(verify(m, &slim, &|n| (n == s.name).then(|| s.data.clone())), Vec::<String>::new());
        // a missing or changed shared file is reported
        assert!(!verify(m, &slim, &|_| None).is_empty());
        let mut t = s.data.clone();
        let len = t.len();
        t[len - 3] ^= 0x40;
        assert!(verify(m, &slim, &|_| Some(t.clone())).iter().any(|x| x.contains("animation") || x.contains("stream")));
    }
}

#[test]
fn chunks_respect_the_16_bit_window() {
    // 12 different 200-frame sequences on 40 bones: ~16 KB of streams each, so several chunks
    let seqs: Vec<(usize, i32)> = (0..12).map(|i| (200, 10 + i)).collect();
    let m = studio::parse(make_model("big", 40, &seqs, 1, 1)).unwrap();
    let d = rewrite::repack(&m).unwrap().expect("smaller");
    let n = studio::parse(d).unwrap();
    assert_eq!(verify(&m, &n, &|_| None), Vec::<String>::new());
    // every offset is a forward u16 from its own entry (read_anim would fail otherwise)
    for q in &n.seqs {
        let a = studio::read_anim(&n.data, q.animindex, 40, q).unwrap();
        for (i, e) in a.entries.iter().enumerate() {
            for r in e.iter().flatten() {
                let at = q.animindex + i * ANIM_SIZE;
                assert!(r.start > at && r.start - at <= layout::MAX_OFFSET);
            }
        }
    }
}

#[test]
fn canonical_streams() {
    // constant channel over 30 and over 50 frames: same canonical bytes
    let s30 = [1u8, 30, 7, 0];
    let s50 = [1u8, 50, 7, 0];
    let c30 = layout::canonical(&s30, &(0..4), 30).unwrap().unwrap();
    let c50 = layout::canonical(&s50, &(0..4), 50).unwrap().unwrap();
    assert_eq!(c30, c50);
    // trailing repeats dropped, values unchanged
    let mut s = vec![4u8, 4];
    for v in [5i16, 6, 6, 6] {
        s.extend(v.to_le_bytes());
    }
    let c = layout::canonical(&s, &(0..s.len()), 4).unwrap().unwrap();
    assert_eq!(c.len(), 2 + 2 * 2);
    assert_eq!(layout::channel_values(&c, &Some(0..c.len()), 4).unwrap(), vec![5, 6, 6, 6]);
    // valid == 0 reads the run header as the value: kept as it is
    let z = [0u8, 3];
    let cz = layout::canonical(&z, &(0..2), 3).unwrap().unwrap();
    assert_eq!(cz, z);
    // all zeros: no stream
    assert!(layout::canonical(&[2u8, 2, 0, 0, 0, 0], &(0..6), 2).unwrap().is_none());
}

#[test]
fn whole_folder() {
    let tmp = std::env::temp_dir().join(format!("mdlopt_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("in");
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("models/player/a/a.mdl", make_model("a", 8, SEQS, 1, 1)),
        ("models/player/b/b.mdl", make_model("b", 8, SEQS, 2, 1)), // same animation as a
        ("models/player/c/c.mdl", make_model("c", 8, &[(40, 5), (30, 6), (40, 5)], 3, 1)),
        ("models/player/gign/gign.mdl", make_model("gign", 8, SEQS, 4, 1)),
        ("models/player/g/g.mdl", make_model("g", 8, SEQS, 5, 2)),
        ("models/player/a/aT.mdl", make_model("aT", 8, &[], 6, 1)),
        ("models/v_knife.mdl", make_model("v_knife", 8, SEQS, 7, 1)),
        ("sound/x.wav", vec![7u8; 5000]),
    ];
    for (r, d) in &files {
        let p = src.join(r);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, d).unwrap();
    }
    let out = tmp.join("out");
    let opts = Options { repack: true, share: true, gzip: Gzip::Best, models_only: false };
    let r = process::run(&src, &out, &opts, &|_, _, _| {}).unwrap();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    let action = |p: &str| r.files.iter().find(|f| f.path == p).map(|f| f.action).unwrap();
    assert_eq!(action("models/player/a/a.mdl"), "slim");
    assert_eq!(action("models/player/b/b.mdl"), "slim");
    assert_eq!(action("models/player/gign/gign.mdl"), "skipped");
    assert_eq!(action("models/player/g/g.mdl"), "skipped");
    assert_eq!(action("models/v_knife.mdl"), "repacked", "weapons are repacked, never split");
    assert_eq!(action("sound/x.wav"), "copied");
    assert_eq!(r.shared.len(), 1);
    assert!(r.lints.iter().any(|l| l.path.ends_with("aT.mdl") && l.message.contains("never loaded")));
    assert!(r.totals.after < r.totals.before && r.totals.after_gz < r.totals.before_gz);
    // every output file and its .gz exist; the manifest lists the shared file
    for f in &r.files {
        let p = out.join(&f.path);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), f.after);
        assert!(p.with_file_name(format!("{}.gz", p.file_name().unwrap().to_string_lossy())).is_file());
    }
    let manifest = std::fs::read_to_string(out.join("shared_anims.txt")).unwrap();
    assert!(manifest.contains(&r.shared[0].name));
    // the written slim model verifies against its original with the written shared file
    let orig = studio::parse(files[0].1.clone()).unwrap();
    let slim = studio::parse(std::fs::read(out.join("models/player/a/a.mdl")).unwrap()).unwrap();
    assert!(verify(&orig, &slim, &|n| std::fs::read(out.join(n)).ok()).is_empty());
    // output equal to the input is refused
    assert!(process::run(&src, &src, &opts, &|_, _, _| {}).is_err());
    let _ = std::fs::remove_dir_all(&tmp);
}
