//! `shrinkfit` command-line tool.
//!
//! ```text
//! shrinkfit [--max-chars 10000] [--format auto|json|yaml] [--indent N] [--report] [FILE|-]
//! ```

mod full_copy;
mod serve;

use shrinkfit::{json, shrink_str, Format, InputFormat, Options};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage: shrinkfit [options] [FILE|-]
       shrinkfit serve [--port N]   (local playground web app)

Shrink a JSON or YAML document to fit within N characters, preserving its schema.

options:
  -n, --max-chars N            output budget in characters (default 10000)
  -f, --format auto|json|yaml  output format (default: auto = same as input)
  -F, --input-format auto|json|yaml  input format (default: auto-detect)
  -i, --indent N               indent width; 0 = compact JSON (default 2)
      --head N                 string head chars shown whole (default 500)
      --section N              chars shown per later string section (default 100)
      --no-keep-last           don't prefer the last item when sampling arrays
      --report                 print a JSON report to stderr

When the output was shrunk, a one-line notice goes to stderr (stdout stays
clean JSON/YAML) saying what was cut and where the full document is: the
input file itself, or, for piped input, a private temp copy.
      --no-notice              don't print the stderr notice (env: SHRINKFIT_NOTICE=0)
      --save-full PATH         write the full input to PATH and reference it
      --no-save-full           don't keep a temp copy of piped input
                               (env: SHRINKFIT_SAVE_FULL=0)
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
    if std::env::args().nth(1).as_deref() == Some("serve") {
        serve::run(std::env::args().skip(2))?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut max = shrinkfit::DEFAULT_MAX_CHARS;
    let mut format = Format::Auto;
    let mut input_format: Option<InputFormat> = None;
    let mut indent: Option<usize> = None;
    let mut head = None;
    let mut section = None;
    let mut keep_last = true;
    let mut report = false;
    let mut notice = !env_off("SHRINKFIT_NOTICE");
    let mut save_full = !env_off("SHRINKFIT_SAVE_FULL");
    let mut save_to: Option<PathBuf> = None;
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
            "-n" | "--max-chars" => max = num(&a, args.next())?,
            "-f" | "--format" => {
                format = match args.next().as_deref() {
                    Some("auto") => Format::Auto,
                    Some("json") => Format::Json,
                    Some("yaml") | Some("yml") => Format::Yaml,
                    _ => return Err("--format must be auto, json or yaml".into()),
                }
            }
            "-F" | "--input-format" => {
                input_format = Some(match args.next().as_deref() {
                    Some("auto") => InputFormat::Auto,
                    Some("json") => InputFormat::Json,
                    Some("yaml") | Some("yml") => InputFormat::Yaml,
                    _ => return Err("--input-format must be auto, json or yaml".into()),
                })
            }
            "-i" | "--indent" => indent = Some(num(&a, args.next())?),
            "--head" => head = Some(num(&a, args.next())?),
            "--section" => section = Some(num(&a, args.next())?),
            "--no-keep-last" => keep_last = false,
            "--report" => report = true,
            "--no-notice" => notice = false,
            "--no-save-full" => save_full = false,
            "--save-full" => {
                save_to = Some(PathBuf::from(
                    args.next().ok_or("--save-full needs a path")?,
                ));
                save_full = true;
            }
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
    opts.format = format;
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
    opts.input_format = input_format.unwrap_or_else(|| match path.as_deref() {
        Some(p) if p.ends_with(".yaml") || p.ends_with(".yml") => InputFormat::Yaml,
        Some(p) if p.ends_with(".json") => InputFormat::Json,
        _ => InputFormat::Auto,
    });

    let out = shrink_str(&input, &opts).map_err(|e| e.to_string())?;
    print!("{}", out.text);
    if out.report.format == Format::Json {
        println!();
    }
    if report {
        eprintln!(
            "{}",
            json::to_string(&out.report.to_value(), json::Layout::Compact)
        );
    }
    if out.report.pruned && notice {
        let ext = if out.report.input_format == InputFormat::Yaml {
            "yaml"
        } else {
            "json"
        };
        let full = full_document(&input, path.as_deref(), ext, save_full, save_to.as_deref());
        eprintln!("{}", notice_line(&out.report, full));
    }
    Ok(if out.report.fits {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// True when an environment variable is set to a "disabled" value.
fn env_off(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .map(|v| v.to_ascii_lowercase())
            .as_deref(),
        Ok("0") | Ok("false") | Ok("no") | Ok("off") | Ok("never")
    )
}

/// Where the full document can be read: the input file itself when there is
/// one (unless `--save-full` asked for a copy), otherwise a saved copy.
fn full_document(
    input: &str,
    path: Option<&str>,
    ext: &str,
    save: bool,
    dest: Option<&std::path::Path>,
) -> Result<Option<PathBuf>, String> {
    if dest.is_none() {
        if let Some(p) = path.filter(|p| *p != "-") {
            let abs = std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
            // Strip Windows' verbatim prefix so the path is usable as printed.
            let shown = abs
                .to_string_lossy()
                .trim_start_matches("\\\\?\\")
                .to_string();
            return Ok(Some(PathBuf::from(shown)));
        }
    }
    if !save {
        return Ok(None);
    }
    full_copy::save(input, ext, dest).map(Some)
}

fn notice_line(report: &shrinkfit::Report, full: Result<Option<PathBuf>, String>) -> String {
    let mut line = format!(
        "shrinkfit: output was shrunk to fit {} chars: {}. Cuts are marked in place with \"…\".",
        report.max_chars,
        report.summary()
    );
    if !report.fits {
        line.push_str(" The budget is smaller than the minimum possible output.");
    }
    match full {
        Ok(Some(p)) => line.push_str(&format!(" Full document: {}", p.display())),
        Ok(None) => {}
        Err(e) => line.push_str(&format!(" (could not save the full document: {e})")),
    }
    line
}
