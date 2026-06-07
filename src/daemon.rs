use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::new_debouncer;

use crate::config::Config;
use crate::integrations;
use crate::paths::{Paths, atomic_write};

pub struct Daemon<'a> {
    paths: &'a Paths,
}

impl<'a> Daemon<'a> {
    pub fn new(paths: &'a Paths) -> Self { Self { paths } }

    pub fn start(&self, config: &Config, foreground: bool) -> Result<()> {
        if let Some(pid) = self.running_pid() {
            return Err(anyhow!("daemon already running (pid {pid})"));
        }
        if !foreground {
            // Re-exec ourselves detached. The grandchild becomes the daemon.
            return self.spawn_detached();
        }
        self.write_pid(std::process::id())?;
        let result = self.run_loop(config);
        let _ = fs::remove_file(self.paths.daemon_pid_file());
        result
    }

    pub fn stop(&self) -> Result<()> {
        let pid = self
            .running_pid()
            .ok_or_else(|| anyhow!("daemon not running"))?;
        kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
            .with_context(|| format!("send SIGTERM to {pid}"))?;
        // Best-effort wait for the pidfile to disappear.
        for _ in 0..50 {
            if self.running_pid().is_none() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Forcefully clean up stale pidfile if process is gone.
        if self.running_pid().is_none() {
            return Ok(());
        }
        Err(anyhow!("daemon did not stop within 5s (pid {pid})"))
    }

    pub fn status(&self) -> Result<()> {
        match self.running_pid() {
            Some(pid) => println!("wallrack daemon running (pid {pid})"),
            None => println!("wallrack daemon not running"),
        }
        Ok(())
    }

    fn running_pid(&self) -> Option<u32> {
        let path = self.paths.daemon_pid_file();
        let raw = fs::read_to_string(&path).ok()?;
        let pid: u32 = raw.trim().parse().ok()?;
        // Verify the process is alive.
        match kill(Pid::from_raw(pid as i32), None) {
            Ok(()) => Some(pid),
            Err(_) => {
                // Stale pidfile — remove so the next start works.
                let _ = fs::remove_file(&path);
                None
            }
        }
    }

    fn write_pid(&self, pid: u32) -> Result<()> {
        self.paths.ensure_cache()?;
        atomic_write(&self.paths.daemon_pid_file(), pid.to_string().as_bytes())
    }

    fn spawn_detached(&self) -> Result<()> {
        // Re-exec the current binary with `daemon start --foreground`,
        // detached via setsid so it survives the parent exiting.
        let exe = std::env::current_exe().context("current_exe")?;
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("daemon").arg("start").arg("--foreground");
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // SAFETY: setsid only mutates the calling process's session id;
        // safe to call between fork and exec.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                nix::unistd::setsid().map(|_| ()).map_err(std::io::Error::from)
            });
        }
        let child = cmd.spawn().context("spawn detached daemon")?;
        println!("wallrack daemon started (pid {})", child.id());
        Ok(())
    }

    fn run_loop(&self, config: &Config) -> Result<()> {
        let integrations = integrations::all();
        // Initial indexing pass so the daemon is immediately useful.
        for integ in &integrations {
            if let Err(err) = integ.index(self.paths, config) {
                eprintln!("wallrack: initial index of {} failed: {err:#}", integ.name());
            }
        }

        // Set up a notify-debouncer-full watcher across every integration's
        // declared watch dirs. We map each watched path back to the integration
        // names that depend on it so a single event only re-indexes what's affected.
        let (tx, rx) = mpsc::channel();
        let mut debouncer = new_debouncer(Duration::from_secs(2), None, tx)
            .context("create watcher")?;

        let mut watched: Vec<(std::path::PathBuf, &'static str)> = Vec::new();
        for integ in &integrations {
            for dir in integ.watch_dirs(config) {
                debouncer
                    .watcher()
                    .watch(&dir, RecursiveMode::Recursive)
                    .with_context(|| format!("watch {}", dir.display()))?;
                watched.push((dir, integ.name()));
            }
        }
        println!("wallrack: watching {} dir(s)", watched.len());

        loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    use std::collections::BTreeSet;
                    let mut affected: BTreeSet<&'static str> = BTreeSet::new();
                    for ev in &events {
                        for path in &ev.paths {
                            for (root, name) in &watched {
                                if path_under(path, root) {
                                    affected.insert(*name);
                                }
                            }
                        }
                    }
                    for name in affected {
                        let Ok(integ) = integrations::by_name(name) else { continue };
                        eprintln!("wallrack: re-indexing {name}");
                        if let Err(err) = integ.index(self.paths, config) {
                            eprintln!("wallrack: re-index of {name} failed: {err:#}");
                        }
                    }
                }
                Ok(Err(errs)) => {
                    for e in errs {
                        eprintln!("wallrack: watch error: {e:?}");
                    }
                }
                Err(_) => break, // channel closed
            }
        }
        Ok(())
    }
}

fn path_under(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}
