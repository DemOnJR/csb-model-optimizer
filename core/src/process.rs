//! A whole game or fast-download folder in, an optimized copy out: models repacked or split,
//! every file with a `.gz` next to it, and a report with before/after sizes.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::gz::{self, Gzip};
use crate::layout::{Job, Pool};
use crate::lint::{self, Lint};
use crate::rewrite::{self, Shared};
use crate::studio::{self, Model, SEQGROUP_SIZE};
use crate::verify;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    /// Pack each model's animation (stream de-duplication inside the model). Safe for any model.
    pub repack: bool,
    /// Move animation shared by several player models into common sequence-group files.
    pub share: bool,
    pub gzip: Gzip,
    /// Only look at models/ (skip maps, sounds, sprites...).
    pub models_only: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { repack: true, share: true, gzip: Gzip::Best, models_only: false }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRow {
    pub path: String,
    pub category: &'static str,
    /// "repacked", "slim", "shared", "copied" or "skipped".
    pub action: &'static str,
    pub before: u64,
    pub after: u64,
    pub before_gz: u64,
    pub after_gz: u64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SharedFile {
    pub name: String,
    pub size: u64,
    pub users: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub files: usize,
    pub before: u64,
    pub after: u64,
    pub before_gz: u64,
    pub after_gz: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub input: String,
    pub output: String,
    pub options: Options,
    pub totals: Totals,
    pub categories: Vec<(String, Totals)>,
    pub files: Vec<FileRow>,
    pub shared: Vec<SharedFile>,
    pub lints: Vec<Lint>,
    /// Verification failures (those models were kept as they were) and read errors.
    pub problems: Vec<String>,
    pub seconds: f64,
}

pub type Progress<'a> = &'a (dyn Fn(&str, usize, usize) + Sync);

fn category(rel: &str) -> &'static str {
    let l = rel.to_ascii_lowercase();
    if l.starts_with("models/player/") || l == "models/player.mdl" {
        "player models"
    } else if l.starts_with("models/") {
        "other models"
    } else if l.starts_with("maps/") {
        "maps"
    } else if l.starts_with("sound/") {
        "sounds"
    } else if l.starts_with("sprites/") {
        "sprites"
    } else {
        "other"
    }
}

fn walk(root: &Path, dir: &Path, skip: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        if p == skip {
            continue;
        }
        let t = e.file_type()?;
        if t.is_dir() {
            walk(root, &p, skip, out)?;
        } else if t.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

/// What happens to one model, decided before anything is written.
enum Plan {
    Copy(String),
    Repack(Vec<u8>),
    Slim(usize, Vec<u8>),
}

/// Groups share candidates whose animation overlaps, as long as each shared file stays no larger
/// than the original animation of every member (so no client spends more memory on a model than
/// today, since each model caches its own copy of its group file) and the group downloads smaller
/// than its members repacked one by one.
fn cluster(
    jobs: &[Vec<Job>],
    anim_bytes: &[usize],
    solo: &[usize],
    slim_len: &[usize],
    pool: &Pool,
    progress: Progress,
) -> Vec<(Vec<usize>, Shared)> {
    let n = jobs.len();
    let ids: Vec<HashSet<u32>> =
        jobs.iter().map(|js| js.iter().flat_map(|j| j.entries.iter().flatten().flatten().copied()).collect()).collect();
    // each sequence table by content (identical sequences share one table in the layout)
    let tables: Vec<Vec<(u64, usize)>> = jobs
        .iter()
        .map(|js| {
            js.iter()
                .map(|j| {
                    let mut h = std::hash::DefaultHasher::new();
                    std::hash::Hash::hash(&j.entries, &mut h);
                    (std::hash::Hasher::finish(&h), j.entries.len() * studio::ANIM_SIZE)
                })
                .collect()
        })
        .collect();
    // overlap between models, in stream bytes they have in common
    let mut users: HashMap<u32, Vec<u32>> = HashMap::new();
    for (m, set) in ids.iter().enumerate() {
        for id in set {
            users.entry(*id).or_default().push(m as u32);
        }
    }
    let mut overlap: HashMap<(u32, u32), u64> = HashMap::new();
    for (id, us) in &users {
        let len = pool.streams[*id as usize].len() as u64;
        if us.len() < 2 || us.len() > 64 || len < 8 {
            continue;
        }
        for (i, a) in us.iter().enumerate() {
            for b in &us[i + 1..] {
                *overlap.entry((*a, *b)).or_default() += len;
            }
        }
    }
    let stream_bytes: Vec<u64> =
        ids.iter().map(|set| set.iter().map(|id| pool.streams[*id as usize].len() as u64).sum()).collect();
    // only pairs that have at least a quarter of the smaller model's streams in common
    let mut pairs: Vec<((u32, u32), u64)> = overlap
        .into_iter()
        .filter(|((a, b), c)| c * 4 >= stream_bytes[*a as usize].min(stream_bytes[*b as usize]))
        .collect();
    pairs.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    // greedy merging
    let mut owner: Vec<usize> = (0..n).collect();
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut built: HashMap<usize, Shared> = HashMap::new();
    let cost = |ms: &[usize], s: Option<&Shared>| -> usize {
        match s {
            Some(s) => s.data.len() + ms.iter().map(|&m| slim_len[m]).sum::<usize>(),
            None => ms.iter().map(|&m| solo[m]).sum(),
        }
    };
    let total = pairs.len();
    for (k, ((a, b), _)) in pairs.into_iter().enumerate() {
        if k % 64 == 0 {
            progress("grouping shared animation", k, total);
        }
        let (ca, cb) = (owner[a as usize], owner[b as usize]);
        if ca == cb {
            continue;
        }
        let mut union = members[ca].clone();
        union.extend(&members[cb]);
        union.sort_unstable();
        let cap = union.iter().map(|&m| anim_bytes[m]).min().unwrap_or(0);
        // quick estimate before the real layout: distinct stream bytes plus tables
        let (mut seen, mut seen_t) = (HashSet::new(), HashSet::new());
        let mut est = 0usize;
        for &m in &union {
            for id in &ids[m] {
                if seen.insert(*id) {
                    est += pool.streams[*id as usize].len();
                }
            }
            for (h, len) in &tables[m] {
                if seen_t.insert(*h) {
                    est += len;
                }
            }
        }
        let before = cost(&members[ca], built.get(&ca)) + cost(&members[cb], built.get(&cb));
        if est > cap || est + union.iter().map(|&m| slim_len[m]).sum::<usize>() >= before {
            continue;
        }
        let refs: Vec<&[Job]> = union.iter().map(|&m| jobs[m].as_slice()).collect();
        let s = rewrite::build_shared(&refs, pool);
        if s.data.len() > cap || cost(&union, Some(&s)) >= before {
            continue;
        }
        built.remove(&cb);
        members[cb].clear();
        for &m in &union {
            owner[m] = ca;
        }
        members[ca] = union;
        built.insert(ca, s);
    }
    let mut out: Vec<(Vec<usize>, Shared)> = built.into_iter().map(|(c, s)| (members[c].clone(), s)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn run(input: &Path, output: &Path, opts: &Options, progress: Progress) -> Result<Report, String> {
    let t0 = Instant::now();
    let input = input.canonicalize().map_err(|e| format!("{}: {e}", input.display()))?;
    fs::create_dir_all(output).map_err(|e| format!("{}: {e}", output.display()))?;
    let output = output.canonicalize().map_err(|e| format!("{}: {e}", output.display()))?;
    if output == input || input.starts_with(&output) {
        return Err("the output folder must not be the input folder or contain it".into());
    }
    progress("listing files", 0, 1);
    let mut rels = Vec::new();
    walk(&input, &input, &output, &mut rels).map_err(|e| e.to_string())?;
    rels.sort();
    let mut problems = Vec::new();
    rels.retain(|r| {
        let l = r.to_ascii_lowercase();
        if opts.models_only && !l.starts_with("models/") {
            return false;
        }
        // existing .gz copies are rebuilt from their raw file
        !(l.ends_with(".gz") && Path::new(&input.join(&r[..r.len() - 3])).is_file())
    });

    progress("reading files", 0, rels.len());
    let datas: Vec<Vec<u8>> = rels
        .par_iter()
        .map(|r| fs::read(input.join(r)).unwrap_or_default())
        .collect();
    let models: Vec<Option<Model>> = rels
        .par_iter()
        .zip(&datas)
        .map(|(r, d)| {
            if r.to_ascii_lowercase().ends_with(".mdl") && d.starts_with(studio::IDST) {
                studio::parse(d.clone()).ok()
            } else {
                None
            }
        })
        .collect();

    let lint_in: Vec<(String, &[u8], Option<&Model>)> =
        rels.iter().zip(&datas).zip(&models).map(|((r, d), m)| (r.clone(), d.as_slice(), m.as_ref())).collect();
    let lints = lint::lint(&lint_in);

    // repack every model that can be (also the fallback for share candidates)
    let eligible: Vec<Option<&'static str>> = rels
        .iter()
        .zip(&models)
        .map(|(r, m)| match m {
            None => Some(if r.to_ascii_lowercase().ends_with(".mdl") { "not a studio v10 model" } else { "" }),
            Some(_) if rewrite::is_stock(r) => Some("stock CS model checked by mp_consistency, every client has it"),
            Some(m) => rewrite::rewritable(m),
        })
        .collect();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let nmodels = models.iter().filter(|m| m.is_some()).count();
    let mut plans: Vec<Plan> = models
        .par_iter()
        .zip(&eligible)
        .map(|(m, why)| {
            let (Some(m), None) = (m, why) else {
                return Plan::Copy(why.unwrap_or("").to_string());
            };
            let k = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            progress("repacking models", k, nmodels);
            if !opts.repack {
                return Plan::Copy(String::new());
            }
            match rewrite::repack(m) {
                Ok(Some(d)) => match studio::parse(d.clone()) {
                    Ok(nm) if verify::verify(m, &nm, &|_| None).is_empty() => {
                        // with gzip on, what clients download is the .gz: keep the original
                        // when the repack would not shrink that too
                        if opts.gzip != Gzip::Off && gz::gzip(&d, opts.gzip).len() >= gz::gzip(&m.data, opts.gzip).len() {
                            Plan::Copy("repack would not make the .gz smaller".into())
                        } else {
                            Plan::Repack(d)
                        }
                    }
                    _ => Plan::Copy("repack failed verification, kept as it was".into()),
                },
                Ok(None) => Plan::Copy("already packed".into()),
                Err(e) => Plan::Copy(format!("left as it was: {e}")),
            }
        })
        .collect();
    for (r, p) in rels.iter().zip(&plans) {
        if let Plan::Copy(why) = p
            && why.contains("verification")
        {
            problems.push(format!("{r}: {why}"));
        }
    }

    // share animation between player models
    let mut shared_files: Vec<(Shared, Vec<usize>)> = Vec::new();
    if opts.share {
        let cands: Vec<usize> = (0..rels.len())
            .filter(|&i| eligible[i].is_none() && rels[i].to_ascii_lowercase().starts_with("models/player/"))
            .collect();
        progress("reading animation streams", 0, cands.len());
        let prepared: Vec<Option<(Pool, rewrite::Prepared)>> = cands
            .par_iter()
            .map(|&i| {
                let mut pool = Pool::default();
                rewrite::prepare(models[i].as_ref().unwrap(), &mut pool).ok().map(|p| (pool, p))
            })
            .collect();
        let mut pool = Pool::default();
        let (mut cidx, mut jobs, mut anim_bytes, mut solo, mut slim_len) = (vec![], vec![], vec![], vec![], vec![]);
        for (&i, p) in cands.iter().zip(prepared) {
            let Some((local, mut prep)) = p else { continue };
            let map: Vec<u32> = local.streams.into_iter().map(|s| pool.intern(s)).collect();
            for j in &mut prep.jobs {
                for e in &mut j.entries {
                    for id in e.iter_mut().flatten() {
                        *id = map[*id as usize];
                    }
                }
            }
            let orig = datas[i].len();
            let cut = prep.region.len() - prep.region.len() % 4;
            cidx.push(i);
            solo.push(match &plans[i] {
                Plan::Repack(d) => d.len(),
                _ => orig,
            });
            slim_len.push(orig - cut + SEQGROUP_SIZE);
            anim_bytes.push(prep.anim_bytes);
            jobs.push(prep.jobs);
        }
        for (members, s) in cluster(&jobs, &anim_bytes, &solo, &slim_len, &pool, progress) {
            let mut slims = Vec::new();
            let mut ok = true;
            for (k, &c) in members.iter().enumerate() {
                let i = cidx[c];
                let m = models[i].as_ref().unwrap();
                let res = rewrite::build_slim(m, &s.name, &s.animindex[k]).and_then(studio::parse);
                match res {
                    Ok(nm) => {
                        let p = verify::verify(m, &nm, &|name| (name == s.name).then(|| s.data.clone()));
                        if p.is_empty() {
                            slims.push((i, nm.data));
                        } else {
                            problems.push(format!("{}: split failed verification ({}), repacked instead", rels[i], p[0]));
                            ok = false;
                        }
                    }
                    Err(e) => {
                        problems.push(format!("{}: split failed ({e}), repacked instead", rels[i]));
                        ok = false;
                    }
                }
            }
            if ok {
                let users: Vec<usize> = slims.iter().map(|(i, _)| *i).collect();
                for (i, d) in slims {
                    plans[i] = Plan::Slim(shared_files.len(), d);
                }
                shared_files.push((s, users));
            }
        }
    }

    // write everything, with .gz copies
    // (path, original bytes, bytes to write, action, note)
    type Out<'a> = (String, &'a [u8], &'a [u8], &'static str, String);
    let mut jobs: Vec<Out> = Vec::new();
    for (i, r) in rels.iter().enumerate() {
        let (out, action, note): (&[u8], &'static str, String) = match &plans[i] {
            Plan::Copy(why) if models[i].is_some() && !why.is_empty() => (&datas[i], "skipped", why.clone()),
            Plan::Copy(_) => (&datas[i], "copied", String::new()),
            Plan::Repack(d) => (d, "repacked", String::new()),
            Plan::Slim(s, d) => (d, "slim", format!("animation in {}", shared_files[*s].0.name)),
        };
        jobs.push((r.clone(), &datas[i], out, action, note));
    }
    for (s, users) in &shared_files {
        let note = format!("used by {} models", users.len());
        jobs.push((s.name.clone(), &[], &s.data, "shared", note));
    }
    let done = std::sync::atomic::AtomicUsize::new(0);
    let total = jobs.len();
    let rows: Vec<Result<FileRow, String>> = jobs
        .par_iter()
        .map(|(rel, before, after, action, note)| {
            progress("writing files", done.fetch_add(1, std::sync::atomic::Ordering::Relaxed), total);
            let dst = output.join(rel);
            if let Some(d) = dst.parent() {
                fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            fs::write(&dst, after).map_err(|e| format!("{}: {e}", dst.display()))?;
            let (mut before_gz, mut after_gz) = (before.len() as u64, after.len() as u64);
            if opts.gzip != Gzip::Off {
                let z = gz::gzip(after, opts.gzip);
                after_gz = z.len() as u64;
                let mut name = dst.clone().into_os_string();
                name.push(".gz");
                fs::write(PathBuf::from(name), &z).map_err(|e| format!("{}.gz: {e}", dst.display()))?;
                before_gz = if before == after { after_gz } else if before.is_empty() { 0 } else { gz::gzip(before, opts.gzip).len() as u64 };
            }
            Ok(FileRow {
                path: rel.clone(),
                category: category(rel),
                action,
                before: before.len() as u64,
                after: after.len() as u64,
                before_gz,
                after_gz,
                note: note.clone(),
            })
        })
        .collect();
    let mut files = Vec::with_capacity(rows.len());
    for r in rows {
        files.push(r?);
    }

    // manifest of shared files for the server's resource list
    let mut shared = Vec::new();
    for (s, users) in &shared_files {
        shared.push(SharedFile {
            name: s.name.clone(),
            size: s.data.len() as u64,
            users: users.iter().map(|&i| rels[i].clone()).collect(),
        });
    }
    if !shared.is_empty() {
        let mut txt = String::from(
            "# Shared animation files made by model-optimizer. Send each one with the models that use it:\n\
             # precache_generic(\"<file>\") next to the skins' precache_model, or one line per file in maps/<map>.res.\n\n",
        );
        for s in &shared {
            txt.push_str(&s.name);
            txt.push('\n');
            for u in &s.users {
                txt.push_str(&format!("#   used by {u}\n"));
            }
        }
        fs::write(output.join("shared_anims.txt"), txt).map_err(|e| e.to_string())?;
    }

    let mut totals = Totals::default();
    let mut cats: Vec<(String, Totals)> = Vec::new();
    for f in &files {
        let i = match cats.iter().position(|c| c.0 == f.category) {
            Some(i) => i,
            None => {
                cats.push((f.category.to_string(), Totals::default()));
                cats.len() - 1
            }
        };
        for t in [&mut totals, &mut cats[i].1] {
            t.files += usize::from(f.action != "shared");
            t.before += f.before;
            t.after += f.after;
            t.before_gz += f.before_gz;
            t.after_gz += f.after_gz;
        }
    }
    cats.sort_by(|a, b| b.1.before.cmp(&a.1.before));
    progress("done", 1, 1);
    let report = Report {
        input: input.display().to_string().trim_start_matches(r"\\?\").to_string(),
        output: output.display().to_string().trim_start_matches(r"\\?\").to_string(),
        options: opts.clone(),
        totals,
        categories: cats,
        files,
        shared,
        lints,
        problems,
        seconds: t0.elapsed().as_secs_f64(),
    };
    Ok(report)
}
