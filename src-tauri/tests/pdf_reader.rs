//! The PDF reader process: the app's own executable started with `--read-pdf`, as the app does
//! for every PDF it reads.

use std::io::Write;
use std::process::{Command, Output, Stdio};

/// A one-page PDF showing `text` in a standard font, with a correct cross-reference table.
fn tiny_pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        pdf.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    pdf
}

fn read(pdf: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_distroclaw"))
        .arg("--read-pdf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(pdf).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn the_app_reads_a_pdf_in_a_process_of_its_own() {
    let out = read(&tiny_pdf("Rechnung Nr 4711"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Rechnung Nr 4711"), "{out:?}");
}

#[test]
fn a_broken_pdf_ends_the_reader_with_a_reason_not_the_app() {
    let out = read(b"%PDF-1.4 nonsense");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(!out.stderr.is_empty());
    // A catalog without pages: the PDF library crashes on it, which counts as a damaged file.
    let no_pages = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Root 1 0 R /Size 2 >>\nstartxref\n44\n%%EOF\n";
    let out = read(no_pages);
    assert!(!out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty());
}
