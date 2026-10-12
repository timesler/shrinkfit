//! The stderr notice and the saved full copy.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn big_json() -> String {
    let items: Vec<String> = (0..300)
        .map(|i| format!(r#"{{"id":{i},"name":"item {i}"}}"#))
        .collect();
    format!(r#"{{"status":"ok","items":[{}]}}"#, items.join(","))
}

fn run(args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_shrinkfit"));
    cmd.args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

#[test]
fn piped_input_notice_points_at_a_saved_full_copy() {
    let input = big_json();
    let out = run(&["-n", "300"], &input, &[]);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert!(stdout.contains("items omitted"), "{stdout}");
    assert!(!stdout.contains("shrinkfit:"), "stdout must stay clean");
    assert!(
        stderr.starts_with("shrinkfit: output was shrunk"),
        "{stderr}"
    );
    assert!(stderr.contains("array items omitted"), "{stderr}");
    let path = stderr
        .split("Full document: ")
        .nth(1)
        .expect(&stderr)
        .trim();
    assert_eq!(std::fs::read_to_string(path).unwrap(), input);
    let _ = std::fs::remove_file(path);
}

#[test]
fn notice_and_copy_are_configurable() {
    let input = big_json();
    // Disabled by flag or environment: nothing on stderr.
    assert!(run(&["-n", "300", "--no-notice"], &input, &[])
        .stderr
        .is_empty());
    let env = [("SHRINKFIT_NOTICE", "0")];
    assert!(run(&["-n", "300"], &input, &env).stderr.is_empty());
    // Notice without a saved copy.
    let out = run(&["-n", "300", "--no-save-full"], &input, &[]);
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("shrunk") && !stderr.contains("Full document"),
        "{stderr}"
    );
    // Explicit destination.
    let dest = std::env::temp_dir().join(format!("shrinkfit-test-{}.json", std::process::id()));
    let out = run(
        &["-n", "300", "--save-full", dest.to_str().unwrap()],
        &input,
        &[],
    );
    assert!(text(&out.stderr).contains(dest.to_str().unwrap()));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), input);
    let _ = std::fs::remove_file(dest);
}

#[test]
fn file_input_is_referenced_not_copied() {
    let input = big_json();
    let file = std::env::temp_dir().join(format!("shrinkfit-in-{}.json", std::process::id()));
    std::fs::write(&file, &input).unwrap();
    let out = run(&["-n", "300", file.to_str().unwrap()], "", &[]);
    let stderr = text(&out.stderr);
    let name = file.file_name().unwrap().to_str().unwrap();
    assert!(
        stderr.contains("Full document: ") && stderr.contains(name),
        "{stderr}"
    );
    let _ = std::fs::remove_file(file);
}

#[test]
fn nothing_is_printed_when_the_output_fits() {
    let out = run(&[], r#"{"a":1}"#, &[]);
    assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
}
