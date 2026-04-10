use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

/// The Oryn star glyph, used as a visual anchor throughout CLI output.
pub const STAR: &str = "✦";

/// Creates a spinner prefixed with the pulsing star glyph.
///
/// The star alternates between ✧ (hollow) and ✦ (filled) to create a
/// subtle breathing effect while work is in progress.
pub fn spinner(message: &str) -> ProgressBar {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::with_template(&format!("  {{spinner:.cyan.bold}} {message}"))
            .unwrap()
            .tick_strings(&["✧", "✦", "✧", "✦", "✦"]),
    );
    sp.enable_steady_tick(Duration::from_millis(140));
    sp
}

/// Creates a small braille spinner for per-file progress, indented
/// beneath the section header.
pub fn file_spinner(filename: &str) -> ProgressBar {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::with_template("    {spinner:.dim} {msg:.dim}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]),
    );
    sp.set_message(filename.to_string());
    sp.enable_steady_tick(Duration::from_millis(80));
    sp
}

/// Prints a section header:  `  ✦ <action> · <count> <noun>(s)`
pub fn header(action: &str, count: usize, noun: &str) {
    println!(
        "  {} {} {} {}",
        style(STAR).cyan().bold(),
        style(action).bold(),
        style("·").dim(),
        style(format!(
            "{count} {noun}{}",
            if count == 1 { "" } else { "s" }
        ))
        .dim(),
    );
}

/// Prints a success summary line:  `  ✦ <message> in <duration>`
pub fn success(message: &str, elapsed: Duration) {
    println!(
        "  {} {} {}",
        style(STAR).green().bold(),
        style(message).green().bold(),
        style(format!("in {}", format_duration(elapsed))).dim(),
    );
}

/// Prints a failure summary line with pass/fail counts and timing.
pub fn failure_summary(passed: usize, failed: usize, elapsed: Duration) {
    println!(
        "  {} {} {} {} {}",
        style(STAR).red().bold(),
        style(format!("{passed} passed")).green(),
        style("·").dim(),
        style(format!("{failed} failed")).red().bold(),
        style(format!("in {}", format_duration(elapsed))).dim(),
    );
}

/// Prints an error line:  `  ✦ error · <message>`
pub fn error(message: &str) {
    eprintln!(
        "  {} {} {} {}",
        style(STAR).red().bold(),
        style("error").red().bold(),
        style("·").dim(),
        style(message).red(),
    );
}

/// Formats a [`Duration`] into a compact human-readable string.
///
/// - Under 1 s → `42ms`
/// - 1 s and above → `1.3s`
pub fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}
