//! `d2-convert convert --install <D2 dir> --out <dir>` and
//! `d2-convert verify --out <dir> [--deep --install <D2 dir>]`.
//!
//! Exit codes: 0 pass, 1 failures, 2 not a D2 LoD install or bad usage.

use std::path::PathBuf;
use std::process::ExitCode;

use d2_convert::{convert, kinds, verify_full, ConvertError, Options, VerifyOptions};

const USAGE: &str = "usage:
  d2-convert convert --install <D2 dir> --out <dir> [--lang ENG] [--force] [--threads N] [--quiet]
  d2-convert verify  --out <dir> [--deep --install <D2 dir>] [--quiet]
(--game is a synonym of --install, --native of --out)";

struct Args {
    install: Option<PathBuf>,
    out: Option<PathBuf>,
    lang: String,
    force: bool,
    deep: bool,
    quiet: bool,
    threads: usize,
    stop_after: Option<usize>,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args {
        install: None,
        out: None,
        lang: "ENG".into(),
        force: false,
        deep: false,
        quiet: false,
        threads: 0,
        stop_after: None,
    };
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--install" | "--game" => a.install = Some(value(&arg)?.into()),
            "--out" | "--native" => a.out = Some(value(&arg)?.into()),
            "--lang" => a.lang = value(&arg)?,
            "--threads" => {
                a.threads = value(&arg)?
                    .parse()
                    .map_err(|_| "--threads: not a number")?
            }
            "--stop-after" => {
                a.stop_after = Some(
                    value(&arg)?
                        .parse()
                        .map_err(|_| "--stop-after: not a number")?,
                )
            }
            "--force" => a.force = true,
            "--deep" => a.deep = true,
            "--quiet" => a.quiet = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let a = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(out) = a.out.clone() else {
        eprintln!("--out is required\n{USAGE}");
        return ExitCode::from(2);
    };
    let kinds = kinds::builtin();
    match cmd.as_str() {
        "convert" => {
            let Some(install) = a.install else {
                eprintln!("--install is required\n{USAGE}");
                return ExitCode::from(2);
            };
            let mut opts = Options::new(install, out);
            opts.language = a.lang;
            opts.force = a.force;
            opts.threads = a.threads;
            opts.progress = !a.quiet;
            opts.stop_after = a.stop_after;
            match convert(&opts, &kinds) {
                Ok(s) => {
                    for (path, check, detail) in &s.failures {
                        eprintln!("FAILED {path}: {check}: {detail}");
                    }
                    ExitCode::from(s.exit_code() as u8)
                }
                Err(ConvertError::NotAnInstall(m)) => {
                    eprintln!("error: {m}");
                    ExitCode::from(2)
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(1)
                }
            }
        }
        "verify" => {
            if a.deep && a.install.is_none() {
                eprintln!("--deep needs --install\n{USAGE}");
                return ExitCode::from(2);
            }
            let opts = VerifyOptions {
                out,
                deep_install: if a.deep { a.install } else { None },
                progress: !a.quiet,
            };
            let outcome = verify_full(&opts, &kinds);
            let problems = outcome.problems;
            for p in &problems {
                eprintln!("FAILED {p}");
            }
            if problems.is_empty() && !a.quiet {
                eprintln!("verify: all files pass");
                if let Some(t) = &outcome.tables {
                    eprintln!("verify: C-TABLE {}", t.line());
                }
            }
            ExitCode::from(u8::from(!problems.is_empty()))
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
