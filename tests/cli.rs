#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gerbrr"))
        .args(args)
        .output()
        .expect("gerbrr runs")
}

#[test]
fn gerbrr_help_has_the_name_tagline_and_commands() {
    let result = run(&["--help"]);
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("Gerbrr — Gerber viewing at ridiculous speed."));
    for command in [
        "gerbrr open",
        "gerbrr inspect",
        "gerbrr screenshot",
        "gerbrr demo-export",
    ] {
        assert!(text.contains(command));
    }
    assert!(!text.contains("Aperture"));
    assert!(!text.contains("gerber-viewer"));
    assert_eq!(run(&["help"]).stdout, text.into_bytes());
}

#[test]
fn gerbrr_version_matches_the_package() {
    let result = run(&["--version"]);
    assert!(result.status.success());
    assert_eq!(
        String::from_utf8(result.stdout).unwrap().trim(),
        format!("gerbrr {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn inspect_subcommand_and_legacy_flag_both_work() {
    for args in [&["inspect"][..], &["--inspect"][..]] {
        let result = run(args);
        assert!(result.status.success(), "{:?}", result.stderr);
        assert!(
            String::from_utf8(result.stdout)
                .unwrap()
                .contains("6 layers")
        );
    }
}

#[test]
fn unknown_flags_are_still_rejected() {
    let result = run(&["--not-a-gerbrr-option"]);
    assert_eq!(result.status.code(), Some(2));
}
