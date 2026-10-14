//! `shrinkfit` command-line tool.
//!
//! ```text
//! shrinkfit [--max-chars 10000] [--format auto|json|yaml] [--indent N] [--report] [FILE|-]
//! ```

mod full_copy;
mod serve;

use shrinkfit::{json, shrink_str, ArrayStrategy, Format, InputFormat, Options, ScorerKind};
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
      --array-strategy sample|condense
                               how big arrays shrink: sample items (default), or keep
                               every item and drop the same low-value fields
      --field-scorer NAME      field ranking for condense: heuristic (default),
                               compact, key_order
      --arguments TEXT|JSON    the tool call's arguments (only their values are used):
                               fields they mention are boosted by condense
      --protect PATTERN        never alter values at this path (repeatable): /results/*/id,
                               ari, content/id. Kept exactly, or the result reports no-fit
      --field-scores JSON|FILE field scores from outside, e.g. from a model: a JSON object
                               of path pattern to 0..1, blended with the built-in score
                               of the fields it names (patterns as for --protect)
      --field-score-weight W   pull of --field-scores against the built-in score, 0..1
                               (default 0.5)
      --explain                print every field's score and keep/drop decision
                               (condense) to stderr
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
            eprintln!("error: {e}");
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
    let mut explain = false;
    let mut protect: Vec<String> = Vec::new();
    let mut arguments: Option<String> = None;
    let mut field_scores: Vec<(String, f64)> = Vec::new();
    let mut field_score_weight: Option<f64> = None;
    let mut array_strategy = ArrayStrategy::Sample;
    let mut field_scorer = ScorerKind::Heuristic;
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
            "--explain" => explain = true,
            "--protect" => protect.push(args.next().ok_or("--protect needs a path pattern")?),
            "--field-scores" => {
                let raw = args
                    .next()
                    .ok_or("--field-scores needs a JSON object or a file")?;
                let text = if raw.trim_start().starts_with('{') {
                    raw
                } else {
                    std::fs::read_to_string(&raw)
                        .map_err(|e| format!("--field-scores {raw}: {e}"))?
                };
                let v = json::parse(&text).map_err(|e| format!("--field-scores: {e}"))?;
                field_scores = shrinkfit::options::parse_field_scores(&v)?;
            }
            "--field-score-weight" => {
                let w: f64 = args
                    .next()
                    .and_then(|x| x.parse().ok())
                    .filter(|x| (0.0..=1.0).contains(x))
                    .ok_or("--field-score-weight needs a number from 0 to 1")?;
                field_score_weight = Some(w);
            }
            "--arguments" => {
                let raw = args.next().ok_or("--arguments needs a value")?;
                arguments = match json::parse(&raw) {
                    Ok(v) => shrinkfit::arguments_text(&v),
                    Err(_) => Some(raw),
                };
            }
            "--array-strategy" => {
                array_strategy = match args.next().as_deref() {
                    Some("sample") => ArrayStrategy::Sample,
                    Some("condense") => ArrayStrategy::Condense,
                    _ => return Err("--array-strategy must be sample or condense".into()),
                }
            }
            "--field-scorer" => {
                field_scorer = args
                    .next()
                    .as_deref()
                    .and_then(ScorerKind::from_name)
                    .ok_or_else(|| {
                        format!("--field-scorer must be one of {:?}", ScorerKind::NAMES)
                    })?
            }
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
    opts.explain = explain;
    opts.protect = protect;
    opts.arguments = arguments;
    opts.field_scores = field_scores;
    if let Some(w) = field_score_weight {
        opts.field_score_weight = w;
    }
    opts.array_strategy = array_strategy;
    opts.field_scorer = field_scorer;
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
    if explain {
        for (path, score, kept, rel) in &out.report.stats.field_decisions {
            let rel = if *rel > 0 {
                format!("  [rel {:.2}]", *rel as f64 / 1000.0)
            } else {
                String::new()
            };
            eprintln!(
                "{:>5.3} {} {path}{rel}",
                *score as f64 / 1000.0,
                if *kept { "keep" } else { "drop" }
            );
        }
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
        "Output was truncated to fit {} chars: {}. Truncated parts are marked in place with \"…\".",
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
