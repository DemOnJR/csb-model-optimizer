//! Problems in a model set that cost download size or break drawing, found without changing
//! anything: texture files that are never loaded, models with no textures at all, duplicate files
//! and duplicate textures inside one model.

use std::collections::HashMap;

use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::studio::{self, Model};

#[derive(Debug, Clone, Serialize)]
pub struct Lint {
    pub path: String,
    /// "warn" (broken or wasted download) or "info".
    pub level: &'static str,
    pub message: String,
    /// Bytes a client would no longer download if this were fixed.
    pub wasted: u64,
}

/// `files`: (relative path, raw bytes, parsed model if it is one).
pub fn lint(files: &[(String, &[u8], Option<&Model>)]) -> Vec<Lint> {
    let mut out = Vec::new();
    let by_lower: HashMap<String, usize> = files.iter().enumerate().map(|(i, f)| (f.0.to_ascii_lowercase(), i)).collect();
    for (path, data, model) in files {
        let lower = path.to_ascii_lowercase();
        let Some(m) = model else { continue };
        // xxxT.mdl: loaded only when xxx.mdl has no textures of its own
        if let Some(main) = lower.strip_suffix("t.mdl").map(|b| format!("{b}.mdl"))
            && let Some(&mi) = by_lower.get(&main)
        {
            let (_, mdata, mmodel) = &files[mi];
            if mmodel.is_some_and(|mm| mm.has_internal_textures()) {
                out.push(Lint {
                    path: path.clone(),
                    level: "warn",
                    message: format!("never loaded: {} has its own textures, so this T file is a wasted download", files[mi].0),
                    wasted: data.len() as u64,
                });
            } else if *mdata == *data {
                out.push(Lint {
                    path: path.clone(),
                    level: "warn",
                    message: format!("identical copy of {}, not a texture file: the model draws without textures", files[mi].0),
                    wasted: data.len() as u64,
                });
            } else if !m.has_internal_textures() {
                out.push(Lint {
                    path: path.clone(),
                    level: "warn",
                    message: "texture file without textures: the model draws without textures".into(),
                    wasted: 0,
                });
            }
            continue;
        }
        // a model with meshes but no textures needs its T file
        if !m.has_internal_textures() && m.h(studio::hdr::NUMBODYPARTS) > 0 {
            let t = lower.strip_suffix(".mdl").map(|b| format!("{b}t.mdl")).unwrap_or_default();
            if !by_lower.contains_key(&t) {
                out.push(Lint {
                    path: path.clone(),
                    level: "warn",
                    message: "no textures inside and no T file next to it: clients fail to load or draw it untextured".into(),
                    wasted: 0,
                });
            }
        }
        // duplicate textures inside the model
        if let Ok(tex) = studio::textures(m) {
            let mut seen: HashMap<&[u8], &str> = HashMap::new();
            let mut dup = 0u64;
            let mut names = Vec::new();
            for (name, r) in &tex {
                let px = &m.data[r.clone()];
                if let Some(first) = seen.get(px) {
                    dup += px.len() as u64;
                    names.push(format!("{name} = {first}"));
                } else {
                    seen.insert(px, name);
                }
            }
            if dup > 0 {
                out.push(Lint {
                    path: path.clone(),
                    level: "info",
                    message: format!("duplicate textures ({}): needs a recompile to share them", names.join(", ")),
                    wasted: dup,
                });
            }
        }
    }
    // byte-identical files under different names
    let mut by_hash: HashMap<[u8; 20], Vec<usize>> = HashMap::new();
    for (i, (_, data, _)) in files.iter().enumerate() {
        if data.len() >= 1024 {
            by_hash.entry(Sha1::digest(data).into()).or_default().push(i);
        }
    }
    let mut groups: Vec<_> = by_hash.into_values().filter(|v| v.len() > 1).collect();
    groups.sort();
    for g in groups {
        let names: Vec<&str> = g.iter().map(|&i| files[i].0.as_str()).collect();
        for &i in &g[1..] {
            out.push(Lint {
                path: files[i].0.clone(),
                level: "info",
                message: format!("same bytes as {} (clients download both)", names[0]),
                wasted: files[i].1.len() as u64,
            });
        }
    }
    out
}
