//! Rayon-friendly progress reporter shared by the image-scanning integrations.
//!
//! Each worker calls `tick()`; rendering is throttled to ~16fps so the
//! terminal isn't slammed, and only one thread holds the render mutex. Uses
//! `\r` + `\x1b[K` (clear-to-EOL) in a single write so the line is updated
//! atomically without flicker. When stderr is not a TTY (rofi script mode),
//! sends `notify-send` notifications with `--replace-id` so a single
//! notification live-updates instead of spamming.

use std::io::{IsTerminal, Write};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

pub struct Progress {
    label: &'static str,
    total: usize,
    done: AtomicUsize,
    last_frame: Mutex<Instant>,
    rendered: AtomicBool,
    tty: bool,
    start: Instant,
    notif_id: Mutex<Option<String>>,
    last_notif: Mutex<Instant>,
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const BAR_WIDTH: usize = 30;
const FRAME_MS: u128 = 60;
const NOTIFY_THROTTLE_MS: u128 = 1_000;

impl Progress {
    pub fn new(label: &'static str, total: usize) -> Self {
        let initial_notif_id = std::env::var("WALLRACK_NOTIF_ID")
            .ok()
            .filter(|s| !s.is_empty());
        Self {
            label,
            total,
            done: AtomicUsize::new(0),
            last_frame: Mutex::new(Instant::now()),
            rendered: AtomicBool::new(false),
            tty: std::io::stderr().is_terminal(),
            start: Instant::now(),
            notif_id: Mutex::new(initial_notif_id),
            last_notif: Mutex::new(Instant::now()),
        }
    }

    pub fn tick(&self) {
        let n = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if !self.tty {
            // try_lock: contending workers skip their tick rather than block.
            let Ok(mut last_notif) = self.last_notif.try_lock() else {
                return;
            };
            let need_first = !self.rendered.load(Ordering::Relaxed);
            if !need_first && last_notif.elapsed().as_millis() < NOTIFY_THROTTLE_MS {
                return;
            }
            *last_notif = Instant::now();
            drop(last_notif);
            self.rendered.store(true, Ordering::Relaxed);
            self.notify_progress(n, false);
            return;
        }
        // try_lock so contending workers drop their tick instead of blocking the
        // par_iter; the next worker past the throttle window will pick it up.
        let Ok(mut last) = self.last_frame.try_lock() else {
            return;
        };
        let need_first = !self.rendered.load(Ordering::Relaxed);
        if !need_first && last.elapsed().as_millis() < FRAME_MS {
            return;
        }
        *last = Instant::now();
        drop(last);
        self.rendered.store(true, Ordering::Relaxed);
        self.render(n);
    }

    fn notify_progress(&self, n: usize, done: bool) {
        let pct = (n * 100)
            .checked_div(self.total)
            .map(|p| p.min(100))
            .unwrap_or(100);
        let body = if done {
            format!("{} index built — {} wallpapers", self.label, n)
        } else {
            format!("Indexing {} — {}/{} ({}%)", self.label, n, self.total, pct)
        };
        let expire_ms = if done { "3000" } else { "0" };

        let mut notif_id = self.notif_id.lock().unwrap();
        let mut cmd = Command::new("notify-send");
        cmd.arg("--print-id")
            .arg(format!("--expire-time={expire_ms}"))
            .arg("Wallrack")
            .arg(&body);
        if let Some(ref id) = *notif_id {
            cmd.arg(format!("--replace-id={id}"));
        }
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let id_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !id_str.is_empty() {
                    *notif_id = Some(id_str);
                }
            }
        }
    }

    fn render(&self, n: usize) {
        let frac = if self.total == 0 {
            1.0
        } else {
            (n as f32 / self.total as f32).min(1.0)
        };
        let filled = (frac * BAR_WIDTH as f32).round() as usize;
        let pct = (frac * 100.0) as u32;
        let spin = SPINNER[(self.start.elapsed().as_millis() / 80) as usize % SPINNER.len()];
        let mut bar = String::with_capacity(BAR_WIDTH * 3);
        for i in 0..BAR_WIDTH {
            bar.push(if i < filled { '█' } else { '░' });
        }
        let line = format!(
            "\rwallrack: {} {} [{}] {}/{} ({}%)\x1b[K",
            self.label, spin, bar, n, self.total, pct
        );
        let stderr = std::io::stderr();
        let mut handle = stderr.lock();
        let _ = handle.write_all(line.as_bytes());
        let _ = handle.flush();
    }

    pub fn finish(&self) {
        let n = self.done.load(Ordering::Relaxed);
        if self.tty {
            // Force a final render in case the last tick was throttled out.
            self.render(n);
            let _ = writeln!(std::io::stderr());
        } else {
            self.notify_progress(n, true);
        }
    }
}
