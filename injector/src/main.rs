//! Injector CLI — thin front-end over the `engine` crate.
//!
//! Usage:
//!   injector list                    -> list visible top-level windows
//!   injector hide  <title-substring> -> hide matching window from capture
//!   injector show  <title-substring> -> restore matching window

use engine::{list_windows, set_hidden};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");

    if cmd == "list" {
        for w in list_windows() {
            println!("[{:>6}] {}", w.pid, w.title);
        }
        return;
    }

    if cmd != "hide" && cmd != "show" {
        eprintln!("usage: injector <list|hide|show> [title-substring]");
        std::process::exit(2);
    }

    let needle = match args.get(2) {
        Some(s) => s.to_lowercase(),
        None => {
            eprintln!("error: '{cmd}' needs a title substring");
            std::process::exit(2);
        }
    };
    let hide = cmd == "hide";

    let target = list_windows()
        .into_iter()
        .find(|w| w.title.to_lowercase().contains(&needle));

    let target = match target {
        Some(w) => w,
        None => {
            eprintln!("no visible window matching '{needle}'");
            std::process::exit(1);
        }
    };

    println!(
        "target: pid={} hwnd={:#x} title=\"{}\"",
        target.pid, target.hwnd, target.title
    );

    match set_hidden(target.pid, target.hwnd, hide) {
        Ok(true) => println!("set_visibility(hide={hide}) -> OK"),
        Ok(false) => println!("set_visibility(hide={hide}) -> FAILED (affinity call returned false)"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
