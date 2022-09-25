//
// regression_test.rs: Check that tool output is as expected.
//
// The standard way of doing Rust integration testing is to use a
// lib.rs file called by main.rs, but I really want to test all the
// way up to binary invocation to ensure missed coverage is minimal,
// so that's what we do here.
//

// TODO: Absolutely minimal-quality replacement for the shell script,
// since I want to rather rewrite how this works.

use std::fs::{self, create_dir_all, remove_dir_all, remove_file, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use test_bin::get_test_bin;

const TEST_TEMP_DIR: &str = "test_tmp";

// Yes, we re-open each time. Minimal change from the shell.
fn log_str(s: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open("test_tmp/test.log")?;
    file.write_all(s.as_bytes())?;
    Ok(())
}

fn log_name(s: &str) -> Result<()> {
    log_str(&format!("=== {}\n", s))
}

#[test]
fn test_regression_old_school() -> Result<()> {
    if Path::new(TEST_TEMP_DIR).exists() {
        remove_dir_all(TEST_TEMP_DIR)?;
    }
    create_dir_all(TEST_TEMP_DIR)?;

    Command::new("sh")
        .args(["-c", &format!("cp testcases/*.pld {}", TEST_TEMP_DIR)])
        .spawn()?
        .wait()?;

    // Special pass for security bit flag test:
    Command::new("sh")
        .args([
            "-c",
            &format!("cp testcases_success/GAL16V8_combinatorial.pld test_tmp/security_bit.pld"),
        ])
        .spawn()?
        .wait()?;

    log_name("security_bit.pld")?;

    get_test_bin("galette")
        .current_dir(TEST_TEMP_DIR)
        .arg("-s")
        .arg("security_bit.pld")
        .spawn()?
        .wait()?;

    let mut names = Vec::new();
    for entry in fs::read_dir(TEST_TEMP_DIR)? {
        let name = entry?.file_name().to_str().unwrap().to_string();
        if name.ends_with(".pld") {
            names.push(name);
        }
    }
    names.sort();

    for name in names.iter() {
        log_name(&name)?;

        let log_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open("test_tmp/test.log")?;
        let log_file2 = log_file.try_clone().unwrap();

        get_test_bin("galette")
            .arg(&name)
            .current_dir(TEST_TEMP_DIR)
            .stdout(log_file)
            .stderr(log_file2)
            .spawn()?
            .wait()?;

        remove_file(&format!("{}/{}", TEST_TEMP_DIR, name))?;
    }

    let diff_res = Command::new("diff")
        .args(["-ru", "baseline", "test_tmp"])
        .status()?;

    assert!(diff_res.success(), "Output generation differs");

    Ok(())
}

#[test]
fn test_successful_generation() -> Result<()> {
    if Path::new("test_temp_success").exists() {
        remove_dir_all("test_temp_success")?;
    }
    create_dir_all("test_temp_success")?;

    Command::new("sh")
        .args([
            "-c",
            &format!("cp testcases_success/*.pld {}", "test_temp_success"),
        ])
        .spawn()?
        .wait()?;

    let mut names = Vec::new();
    for entry in fs::read_dir("test_temp_success")? {
        let name = entry?.file_name().to_str().unwrap().to_string();
        if name.ends_with(".pld") {
            names.push(name);
        }
    }
    names.sort();

    for name in names.iter() {
        let results = get_test_bin("galette")
            .current_dir("test_temp_success")
            .arg(name)
            .output()?;

        assert!(results.status.success(), "'{:?}' did not succeed", name);
        assert_eq!(
            results.stdout.len(),
            0,
            "'{:?}' produced unexpected output to stdout: {:?}",
            name,
            results.stdout
        );
        assert_eq!(
            results.stderr.len(),
            0,
            "'{:?}' produced unexpected output to stderr: {:?}",
            name,
            results.stderr
        );

        remove_file(&format!("{}/{}", "test_temp_success", name))?;
    }

    let diff_res = Command::new("diff")
        .args(["-ru", "baseline_success", "test_temp_success"])
        .status()?;

    assert!(diff_res.success(), "Output generation differs");

    Ok(())
}