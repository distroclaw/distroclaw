// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--self-test") => std::process::exit(distroclaw_lib::self_test(args.get(2).map(Into::into))),
        // The app reads PDFs in a process of its own (see `attachments`).
        Some(distroclaw_lib::PDF_READER_FLAG) => std::process::exit(distroclaw_lib::pdf_reader_main()),
        _ => distroclaw_lib::run(),
    }
}
