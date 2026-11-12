//! `fanta-check <project-dir> [--json]`: everything wrong with a project,
//! without writing anything (spec 0009 R1). Exit status 0 when there are no
//! errors, 1 when there are, 2 when the directory is not a project.

use fanta_format::{SourceDiagnostic, SourceSeverity};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut json = false;
    let mut dir = None;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--json" => json = true,
            _ if dir.is_none() => dir = Some(argument),
            _ => {
                eprintln!("usage: fanta-check <project-dir> [--json]");
                return ExitCode::from(2);
            }
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage: fanta-check <project-dir> [--json]");
        return ExitCode::from(2);
    };
    let findings = match fanta_format::check_project(Path::new(&dir)) {
        Ok(findings) => findings,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    if json {
        match fanta_format::findings_json(&findings) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(2);
            }
        }
    } else {
        for finding in &findings {
            println!("{}", line(finding));
        }
    }
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == SourceSeverity::Error)
        .count();
    if !json {
        println!("{errors} errors, {} warnings", findings.len() - errors);
    }
    if errors == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

/// `file:line:column: severity[code]: message`, the shape editors and CI
/// annotations already parse.
fn line(finding: &SourceDiagnostic) -> String {
    let mut place = finding.file.clone().unwrap_or_default();
    if let Some(line) = finding.line {
        place.push_str(&format!(":{line}"));
        if let Some(column) = finding.column {
            place.push_str(&format!(":{column}"));
        }
    }
    if !place.is_empty() {
        place.push_str(": ");
    }
    let severity = match finding.severity {
        SourceSeverity::Error => "error",
        SourceSeverity::Warning => "warning",
        SourceSeverity::Info => "info",
    };
    format!("{place}{severity}[{}]: {}", finding.code, finding.message)
}
