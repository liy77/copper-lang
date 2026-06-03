//! Terminal styling for cforge — the Rust counterpart of `scripts/_pretty.py`,
//! so the compiler's output shares the look of the Python tooling: a copper
//! accent, box-drawn banner, `✔ / ✗ / ▸ / ·` status glyphs, and a spinner for
//! long-running steps. Everything degrades to plain ASCII when stdout isn't a
//! TTY or `NO_COLOR` is set.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use colored::Colorize;

/// Copper accent (matches `_pretty.py`'s 256-colour 208).
fn copper(s: &str) -> String {
    if color() {
        s.truecolor(255, 135, 0).bold().to_string()
    } else {
        s.to_string()
    }
}

fn dim(s: &str) -> String {
    if color() {
        s.truecolor(150, 150, 150).to_string()
    } else {
        s.to_string()
    }
}

/// Colour is on when stdout is a terminal and `NO_COLOR` isn't set.
fn color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

fn unicode() -> bool {
    // Best-effort: assume UTF-8 on a TTY; cforge already prints emoji elsewhere.
    std::io::stdout().is_terminal()
}

/// A boxed banner: title with an optional subtitle, framed in copper.
pub fn banner(title: &str, subtitle: &str) {
    let (tl, tr, bl, br, h, v) = if unicode() {
        ("╭", "╮", "╰", "╯", "─", "│")
    } else {
        ("+", "+", "+", "+", "-", "|")
    };
    let inner = title.len().max(subtitle.len()) + 2;
    let bar = h.repeat(inner);
    println!("{}", copper(&format!("{tl}{bar}{tr}")));
    println!(
        "{} {} {}",
        copper(v),
        pad(&copper(title), inner - 1),
        copper(v)
    );
    if !subtitle.is_empty() {
        println!(
            "{} {} {}",
            copper(v),
            pad(&dim(subtitle), inner - 1),
            copper(v)
        );
    }
    println!("{}", copper(&format!("{bl}{bar}{br}")));
}

fn pad(s: &str, _width: usize) -> String {
    // Padding by display width is approximate (ANSI codes inflate len); keep it
    // simple — the box doesn't need to be pixel-perfect.
    s.to_string()
}

/// The copper arrow glyph, for inline use (e.g. "▸ name v1.2").
pub fn arrow() -> String {
    copper(if unicode() { "▸" } else { ">" })
}

/// A section heading (▸ copper).
pub fn head(msg: &str) {
    println!("{} {}", arrow(), msg.bold());
}

/// A sub-step (· dim).
pub fn step(msg: &str) {
    let g = if unicode() { "·" } else { "-" };
    println!("  {} {}", dim(g), msg);
}

/// Success line (✔ green).
pub fn ok(msg: &str) {
    let g = if unicode() { "✔" } else { "+" };
    println!("{} {}", g.green().bold(), msg);
}

/// Warning line (! yellow).
pub fn warn(msg: &str) {
    println!("{} {}", "!".yellow().bold(), msg);
}

/// Failure line (✗ red).
pub fn fail(msg: &str) {
    let g = if unicode() { "✗" } else { "x" };
    eprintln!("{} {}", g.red().bold(), msg);
}

// ---- file tree ----

#[derive(Default)]
struct TreeNode {
    children: BTreeMap<String, TreeNode>,
}

/// Print a file tree under `label`. `paths` are display paths (relative to the
/// project) using `/` or `\` separators; common directories are merged into
/// branches with `├─ / └─ / │` connectors.
pub fn tree(label: &str, paths: &[String]) {
    head(label);
    let mut root = TreeNode::default();
    for p in paths {
        let mut node = &mut root;
        for part in p.split(['/', '\\']).filter(|s| !s.is_empty()) {
            node = node.children.entry(part.to_string()).or_default();
        }
    }
    render_tree(&root, "");
}

fn render_tree(node: &TreeNode, prefix: &str) {
    let (tee, ell, bar) = if unicode() {
        ("├─ ", "└─ ", "│  ")
    } else {
        ("|- ", "`- ", "|  ")
    };
    let n = node.children.len();
    for (i, (name, child)) in node.children.iter().enumerate() {
        let last = i == n - 1;
        let connector = if last { ell } else { tee };
        let is_dir = !child.children.is_empty();
        let shown = if is_dir {
            copper(&format!("{name}/"))
        } else {
            name.clone()
        };
        println!("  {}{}{}", dim(prefix), dim(connector), shown);
        let next = format!("{prefix}{}", if last { "   " } else { bar });
        render_tree(child, &next);
    }
}

// ---- progress bar (determinate) ----

/// A determinate progress bar for a step with a known count (downloading N
/// dependencies, writing N files). On a non-TTY it prints discrete `· item`
/// lines instead of animating, so logs stay readable.
pub struct ProgressBar {
    total: usize,
    label: String,
    animated: bool,
    width: usize,
}

impl ProgressBar {
    pub fn new(total: usize, label: &str) -> Self {
        Self {
            total: total.max(1),
            label: label.to_string(),
            animated: color(),
            width: 22,
        }
    }

    /// Redraw the bar at `current`/total with the current `item` shown.
    pub fn set(&self, current: usize, item: &str) {
        let cur = current.min(self.total);
        if !self.animated {
            // Non-TTY: one line per item so logs are clean.
            if !item.is_empty() {
                println!(
                    "  {} {} ({}/{})",
                    dim(if unicode() { "·" } else { "-" }),
                    item,
                    cur,
                    self.total
                );
            }
            return;
        }
        let filled = self.width * cur / self.total;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(self.width - filled));
        let bar = if color() {
            bar.truecolor(255, 135, 0).to_string()
        } else {
            bar
        };
        print!(
            "\r\x1b[2K{} [{}] {}/{}  {}",
            self.label,
            bar,
            cur,
            self.total,
            dim(item)
        );
        let _ = std::io::stdout().flush();
    }

    /// Finish: clear the bar and print a success line.
    pub fn finish(self, msg: &str) {
        if self.animated {
            print!("\r\x1b[2K");
            let _ = std::io::stdout().flush();
        }
        ok(msg);
    }
}

/// An animated spinner for a long-running step. Drop it (or call `done`) to
/// stop. On a non-TTY it just prints the label once and animates nothing, so
/// logs and CI stay clean.
pub struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    /// Live label, updatable mid-spin via [`Spinner::set`] (e.g. the crate
    /// currently compiling), read by the animation thread each frame.
    label: Arc<Mutex<String>>,
    animated: bool,
}

impl Spinner {
    /// Start spinning with `label`.
    pub fn start(label: &str) -> Self {
        let animated = color();
        let label = Arc::new(Mutex::new(label.to_string()));
        if !animated {
            if let Ok(l) = label.lock() {
                println!("  {} {}", dim(if unicode() { "·" } else { "-" }), *l);
            }
            return Self {
                stop: Arc::new(AtomicBool::new(true)),
                handle: None,
                label,
                animated,
            };
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = stop.clone();
        let label_t = label.clone();
        let handle = std::thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0usize;
            while !stop_t.load(Ordering::Relaxed) {
                let f = frames[i % frames.len()];
                let text = label_t.lock().map(|s| s.clone()).unwrap_or_default();
                // `\x1b[2K` clears the line so a shorter label leaves no tail.
                print!("\r\x1b[2K{} {} ", f.truecolor(255, 135, 0), text);
                let _ = std::io::stdout().flush();
                i += 1;
                std::thread::sleep(Duration::from_millis(80));
            }
        });
        Self {
            stop,
            handle: Some(handle),
            label,
            animated,
        }
    }

    /// Update the spinner's label while it spins (e.g. the current crate).
    pub fn set(&self, label: &str) {
        if let Ok(mut l) = self.label.lock() {
            *l = label.to_string();
        }
    }

    /// A clonable handle to the live label, so another thread (e.g. one reading
    /// `cargo`'s output) can update what the spinner shows.
    pub fn label_handle(&self) -> Arc<Mutex<String>> {
        self.label.clone()
    }

    fn stop_thread(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        if self.animated {
            // Clear the spinner line.
            print!("\r\x1b[2K");
            let _ = std::io::stdout().flush();
        }
    }

    /// Stop and replace the spinner with a success line.
    pub fn done(mut self, msg: &str) {
        self.stop_thread();
        ok(msg);
    }

    /// Stop and replace the spinner with a failure line.
    pub fn fail(mut self, msg: &str) {
        self.stop_thread();
        fail(msg);
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        // Make sure the thread is stopped even if neither done/fail was called.
        if self.handle.is_some() {
            self.stop_thread();
            // Leave the label as a plain step so the user still sees what ran.
            if let Ok(l) = self.label.lock() {
                step(&l);
            }
        }
    }
}
