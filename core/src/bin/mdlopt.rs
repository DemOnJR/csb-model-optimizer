//! Command-line front end of the model optimizer (the app uses the same library).
//!
//! mdlopt run INPUT_DIR OUTPUT_DIR [--no-repack] [--no-share] [--gzip off|best|zopfli] [--models-only] [--report FILE]
//! mdlopt verify ORIGINAL.mdl NEW.mdl [GROUP_DIR]

use std::path::Path;
use std::process::ExitCode;

use csb_mdl::gz::Gzip;
use csb_mdl::process::{self, Options};
use csb_mdl::{studio, verify};

fn usage() -> ExitCode {
    eprintln!("usage: mdlopt run INPUT_DIR OUTPUT_DIR [--no-repack] [--no-share] [--gzip off|best|zopfli] [--models-only] [--report FILE]");
    eprintln!("       mdlopt verify ORIGINAL.mdl NEW.mdl [GAME_DIR]");
    ExitCode::from(2)
}

fn mb(b: u64) -> String {
    format!("{:.2} MB", b as f64 / 1_048_576.0)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") if args.len() >= 3 => {
            let mut opts = Options::default();
            let mut report_path = None;
            let mut it = args[3..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--no-repack" => opts.repack = false,
                    "--no-share" => opts.share = false,
                    "--models-only" => opts.models_only = true,
                    "--gzip" => {
                        opts.gzip = match it.next().map(String::as_str) {
                            Some("off") => Gzip::Off,
                            Some("best") => Gzip::Best,
                            Some("zopfli") => Gzip::Zopfli,
                            _ => return usage(),
                        }
                    }
                    "--report" => report_path = it.next().cloned(),
                    _ => return usage(),
                }
            }
            let last = std::sync::Mutex::new(String::new());
            let progress = |stage: &str, _done: usize, _total: usize| {
                let mut l = last.lock().unwrap();
                if *l != stage {
                    eprintln!("{stage}...");
                    *l = stage.to_string();
                }
            };
            let r = match process::run(Path::new(&args[1]), Path::new(&args[2]), &opts, &progress) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            for (c, t) in &r.categories {
                println!(
                    "{c:>14}: {:>5} files  raw {} -> {}   gz {} -> {}",
                    t.files, mb(t.before), mb(t.after), mb(t.before_gz), mb(t.after_gz)
                );
            }
            let t = &r.totals;
            println!(
                "{:>14}: {:>5} files  raw {} -> {} ({:.1}%)   gz {} -> {} ({:.1}% of raw before)",
                "total", t.files, mb(t.before), mb(t.after), 100.0 * t.after as f64 / t.before.max(1) as f64,
                mb(t.before_gz), mb(t.after_gz), 100.0 * t.after_gz as f64 / t.before.max(1) as f64
            );
            let count = |a: &str| r.files.iter().filter(|f| f.action == a).count();
            println!(
                "repacked {}, slim {}, shared files {}, skipped models {}, {:.1} s",
                count("repacked"), count("slim"), count("shared"), count("skipped"), r.seconds
            );
            for l in &r.lints {
                println!("{} {}: {}{}", l.level, l.path, l.message,
                    if l.wasted > 0 { format!(" ({} bytes)", l.wasted) } else { String::new() });
            }
            for p in &r.problems {
                println!("PROBLEM {p}");
            }
            if let Some(p) = report_path {
                std::fs::write(p, serde_json::to_vec_pretty(&r).unwrap()).unwrap();
            }
            if r.problems.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        Some("verify") if args.len() >= 3 => {
            let load = |p: &str| std::fs::read(p).map_err(|e| e.to_string()).and_then(|d| studio::parse(d).map_err(|e| e.0));
            let (o, n) = match (load(&args[1]), load(&args[2])) {
                (Ok(o), Ok(n)) => (o, n),
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let root = args.get(3).cloned().unwrap_or_else(|| ".".into());
            let p = verify::verify(&o, &n, &|name| std::fs::read(Path::new(&root).join(name)).ok());
            for x in &p {
                println!("{x}");
            }
            println!("verify: {}", if p.is_empty() { "OK".to_string() } else { format!("{} problem(s)", p.len()) });
            if p.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        _ => usage(),
    }
}
