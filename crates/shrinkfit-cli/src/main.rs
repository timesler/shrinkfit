//! `shrinkfit` command-line tool.
//!
//! ```text
//! shrinkfit --max-chars 4000 [--format json|yaml] [--indent N] [--report] [FILE|-]
//! ```

use shrinkfit::{json, shrink, Format, Options};
use std::io::Read;
use std::process::ExitCode;

const USAGE: &str = "\
usage: shrinkfit --max-chars N [options] [FILE|-]

Shrink a JSON document to fit within N characters, preserving its schema.

options:
  -n, --max-chars N            output budget in characters (required)
  -f, --format json|yaml       output format (default: json)
  -i, --indent N               indent width; 0 = compact JSON (default 0; yaml 2)
      --head N                 string head chars shown whole (default 500)
      --section N              chars shown per later string section (default 100)
      --no-keep-last           don't prefer the last item when sampling arrays
      --report                 print a JSON report to stderr
  -h, --help                   show this help";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("shrinkfit: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let mut max: Option<usize> = None;
    let mut format = Format::Json;
    let mut indent: Option<usize> = None;
    let mut head = None;
    let mut section = None;
    let mut keep_last = true;
    let mut report = false;
    let mut path: Option<String> = None;

    let num = |flag: &str, v: Option<String>| -> Result<usize, String> {
        v.ok_or_else(|| format!("{flag} needs a value"))?
            .parse()
            .map_err(|_| format!("{flag} must be a non-negative integer"))
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "-n" | "--max-chars" => max = Some(num(&a, args.next())?),
            "-f" | "--format" => {
                format = match args.next().as_deref() {
                    Some("json") => Format::Json,
                    Some("yaml") | Some("yml") => Format::Yaml,
                    _ => return Err("--format must be json or yaml".into()),
                }
            }
            "-i" | "--indent" => indent = Some(num(&a, args.next())?),
            "--head" => head = Some(num(&a, args.next())?),
            "--section" => section = Some(num(&a, args.next())?),
            "--no-keep-last" => keep_last = false,
            "--report" => report = true,
            s if s.starts_with('-') && s != "-" => {
                return Err(format!("unknown option {s}\n\n{USAGE}"))
            }
            s => {
                if path.replace(s.to_string()).is_some() {
                    return Err("only one input file may be given".into());
                }
            }
        }
    }
    let max = max.ok_or_else(|| format!("--max-chars is required\n\n{USAGE}"))?;

    let mut input = String::new();
    match path.as_deref() {
        None | Some("-") => std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?,
        Some(p) => {
            input = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            0
        }
    };

    let mut opts = Options::new(max);
    if format == Format::Yaml {
        opts = opts.yaml();
    }
    if let Some(i) = indent {
        if format == Format::Yaml && i < 2 {
            return Err("YAML indent must be at least 2".into());
        }
        opts.indent = i;
    }
    if let Some(h) = head {
        opts.text.head = h;
    }
    if let Some(s) = section {
        opts.text.section = s;
    }
    opts.keep_last_item = keep_last;

    let value = json::parse(&input).map_err(|e| format!("invalid JSON input: {e}"))?;
    let out = shrink(&value, &opts);
    print!("{}", out.text);
    if format == Format::Json {
        println!();
    }
    if report {
        eprintln!(
            "{}",
            json::to_string(&out.report.to_value(), json::Layout::Compact)
        );
    }
    Ok(if out.report.fits {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
