//! Running the Python analyzer from the app: spawn
//! `qwertone-analyze FILE --json-progress`, read stderr line by line, and
//! turn the stage events into a progress bar. The last event is
//! `{"event": "done", "song_dir": …}`.

use crossbeam_channel::{Receiver, Sender, unbounded};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What the analyzer said.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Stage {
        stage: String,
        message: String,
    },
    Done {
        song_dir: PathBuf,
        skipped: bool,
    },
    Error(String),
    /// The lyrics step finished (`status`: synced | plain | instrumental |
    /// not_found).
    Lyrics {
        status: String,
        language: Option<String>,
    },
    /// A line that wasn't JSON (warnings from libraries).
    Log(String),
    /// The process ended (exit code, None if killed).
    Exit(Option<i32>),
}

/// Parse one stderr line.
pub fn parse_line(line: &str) -> Event {
    let line = line.trim();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Event::Log(line.to_string());
    };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    match v.get("event").and_then(|e| e.as_str()) {
        Some("stage") => Event::Stage {
            stage: s("stage"),
            message: s("message"),
        },
        Some("done") => Event::Done {
            song_dir: PathBuf::from(s("song_dir")),
            skipped: v.get("skipped").and_then(|x| x.as_bool()).unwrap_or(false),
        },
        Some("error") => Event::Error(s("message")),
        Some("lyrics") => Event::Lyrics {
            status: s("status"),
            language: v.get("language").and_then(|x| x.as_str()).map(|x| x.to_string()),
        },
        _ => Event::Log(line.to_string()),
    }
}

/// What to ask the analyzer for.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// chords | melody | chords+melody | full | bass | piano | drums | simplified
    pub mode: String,
    /// Add easy parts (difficulty 1–5), or None.
    pub simplify: Option<u32>,
    /// Skip stem separation (fast, less accurate).
    pub no_separate: bool,
    pub force: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            mode: "chords+melody".into(),
            simplify: Some(2),
            no_separate: false,
            force: false,
        }
    }
}

pub const MODES: [(&str, &str); 8] = [
    ("chords+melody", "Chords + melody (best start)"),
    ("chords", "Chords only"),
    ("melody", "Melody only (the singing)"),
    ("simplified", "Easy version"),
    ("piano", "Piano notes"),
    ("bass", "Bass line"),
    ("drums", "Drums"),
    ("full", "Every instrument"),
];

impl Options {
    pub fn args(&self, file: &Path) -> Vec<String> {
        let mut a = vec![
            file.display().to_string(),
            "--json-progress".to_string(),
            "--mode".into(),
            self.mode.clone(),
        ];
        if let Some(d) = self.simplify {
            if self.mode == "simplified" {
                a.push("-d".into());
            } else {
                a.push("--simplify".into());
                a.push("-d".into());
            }
            a.push(d.clamp(1, 5).to_string());
        }
        if self.no_separate {
            a.push("--no-separate".into());
        }
        if self.force {
            a.push("--force".into());
        }
        a
    }
}

/// The `qwertone-analyze` program: on PATH, or in ~/.local/bin (where
/// setup.sh links it; GUI apps don't always have that on PATH).
pub fn find_analyzer() -> Option<PathBuf> {
    let name = "qwertone-analyze";
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            let p = d.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let local = dirs::home_dir()?.join(".local/bin").join(name);
    local.is_file().then_some(local)
}

/// Rough share of the total time each stage takes, so the bar moves sensibly.
fn stage_weight(stage: &str) -> f32 {
    match stage {
        "input" => 1.0,
        "decode" => 3.0,
        "models" => 10.0,
        "stems" => 45.0,
        "features" => 6.0,
        "rhythm" => 6.0,
        "chords" => 5.0,
        "key" => 1.0,
        "sections" => 3.0,
        "melody" | "pitch" => 12.0,
        "full" => 30.0,
        "drums" => 4.0,
        "bass" => 8.0,
        "piano" => 25.0,
        "midi" => 2.0,
        "write" => 2.0,
        _ => 2.0,
    }
}

/// The stages a run will go through, in order.
pub fn expected_stages(opts: &Options, midi_input: bool) -> Vec<&'static str> {
    if midi_input {
        return vec!["input", "midi", "rhythm", "key", "write"];
    }
    let mut v = vec!["input", "decode"];
    if !opts.no_separate {
        v.push("stems");
    }
    v.extend(["features", "rhythm", "chords", "key", "sections"]);
    match opts.mode.as_str() {
        "chords" => {}
        "melody" | "chords+melody" | "simplified" => v.push("melody"),
        "full" => v.extend(["full", "drums"]),
        "bass" => v.push("bass"),
        "piano" => v.push("piano"),
        "drums" => v.push("drums"),
        _ => v.push("melody"),
    }
    v.push("write");
    v
}

/// Progress 0..1 given the furthest stage reached and how long it has run.
pub fn fraction(expected: &[&str], current: Option<&str>, secs_in_stage: f32) -> f32 {
    let total: f32 = expected
        .iter()
        .map(|s| stage_weight(s))
        .sum::<f32>()
        .max(1.0);
    let Some(cur) = current else { return 0.0 };
    let idx = expected.iter().position(|s| *s == cur);
    let Some(idx) = idx else {
        return 0.0;
    };
    let before: f32 = expected[..idx].iter().map(|s| stage_weight(s)).sum();
    let w = stage_weight(cur);
    // Creep through the stage: half way after ~w seconds, never past it.
    let inside = w * (1.0 - (-secs_in_stage / w.max(1.0) * 0.7).exp());
    ((before + inside) / total).clamp(0.0, 0.99)
}

/// A running (or finished) analysis.
pub struct Job {
    pub file: PathBuf,
    pub opts: Options,
    pub started: Instant,
    rx: Receiver<Event>,
    child: Arc<Mutex<Option<Child>>>,
    pub expected: Vec<&'static str>,
    pub stage: Option<String>,
    pub stage_started: Instant,
    pub message: String,
    pub log: Vec<String>,
    pub result: Option<Result<PathBuf, String>>,
    /// Lyrics jobs: a clean exit means done (no "done" event), with this folder.
    ok_on_exit: Option<PathBuf>,
    /// What the lyrics step found.
    pub lyrics_status: Option<String>,
}

impl Job {
    /// Start `qwertone-analyze` on a file.
    pub fn start(program: &Path, file: &Path, opts: Options) -> Result<Job, String> {
        let midi = super::library::is_midi(file);
        let expected = expected_stages(&opts, midi);
        Job::spawn(program, opts.args(file), file, opts, expected, None)
    }

    /// `qwertone-analyze lyrics <song dir> --json-progress [extra…]`: find
    /// (or rebuild) the lyrics of an analysed song.
    pub fn lyrics(program: &Path, song_dir: &Path, extra: &[&str]) -> Result<Job, String> {
        let mut args = vec![
            "lyrics".to_string(),
            song_dir.display().to_string(),
            "--json-progress".into(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        Job::spawn(program, args, song_dir, Options::default(), vec!["lyrics"], Some(song_dir.to_path_buf()))
    }

    /// `qwertone-analyze import-lrc <file> <song dir> --json-progress`.
    pub fn import_lrc(program: &Path, lrc: &Path, song_dir: &Path) -> Result<Job, String> {
        let args = vec![
            "import-lrc".to_string(),
            lrc.display().to_string(),
            song_dir.display().to_string(),
            "--json-progress".into(),
        ];
        Job::spawn(program, args, song_dir, Options::default(), vec!["lyrics"], Some(song_dir.to_path_buf()))
    }

    fn spawn(
        program: &Path,
        args: Vec<String>,
        file: &Path,
        opts: Options,
        expected: Vec<&'static str>,
        ok_on_exit: Option<PathBuf>,
    ) -> Result<Job, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't start {}: {e}", program.display()))?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let mut stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, rx) = unbounded();
        let child = Arc::new(Mutex::new(Some(child)));
        // stdout only has the folder path; drain it so the pipe never fills.
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
        });
        let waiter = child.clone();
        std::thread::spawn(move || read_events(stderr, tx, waiter));
        Ok(Job {
            expected,
            file: file.to_path_buf(),
            opts,
            started: Instant::now(),
            rx,
            child,
            stage: None,
            stage_started: Instant::now(),
            message: "starting…".into(),
            log: Vec::new(),
            result: None,
            ok_on_exit,
            lyrics_status: None,
        })
    }

    /// Read new events. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ev) = self.rx.try_recv() {
            changed = true;
            match ev {
                Event::Stage { stage, message } => {
                    if stage == "done" {
                        continue;
                    }
                    if self.stage.as_deref() != Some(stage.as_str()) {
                        if !self.expected.contains(&stage.as_str()) {
                            // An unexpected stage: keep the bar where it is.
                        } else {
                            self.stage = Some(stage.clone());
                            self.stage_started = Instant::now();
                        }
                    }
                    self.message = message;
                }
                Event::Done { song_dir, .. } => {
                    self.result = Some(Ok(song_dir));
                }
                Event::Error(m) => {
                    self.result.get_or_insert(Err(m));
                }
                Event::Lyrics { status, .. } => {
                    self.lyrics_status = Some(status);
                }
                Event::Log(l) => {
                    if !l.is_empty() {
                        self.log.push(l);
                        if self.log.len() > 200 {
                            self.log.remove(0);
                        }
                    }
                }
                Event::Exit(code) => {
                    if self.result.is_none()
                        && code == Some(0)
                        && let Some(d) = &self.ok_on_exit
                    {
                        self.result = Some(Ok(d.clone()));
                    }
                    if self.result.is_none() {
                        let last = self
                            .log
                            .iter()
                            .rev()
                            .find(|l| !l.trim().is_empty())
                            .cloned()
                            .unwrap_or_default();
                        self.result = Some(Err(match code {
                            Some(c) => format!("the analyzer stopped (exit code {c}). {last}"),
                            None => "stopped".into(),
                        }));
                    }
                }
            }
        }
        changed
    }

    pub fn running(&self) -> bool {
        self.result.is_none()
    }

    pub fn fraction(&self) -> f32 {
        if matches!(self.result, Some(Ok(_))) {
            return 1.0;
        }
        fraction(
            &self.expected,
            self.stage.as_deref(),
            self.stage_started.elapsed().as_secs_f32(),
        )
    }

    pub fn cancel(&mut self) {
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
        self.result.get_or_insert(Err("cancelled".into()));
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.running() {
            self.cancel();
        }
    }
}

fn read_events(
    stderr: std::process::ChildStderr,
    tx: Sender<Event>,
    child: Arc<Mutex<Option<Child>>>,
) {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if tx.send(parse_line(&line)).is_err() {
            break;
        }
    }
    // stderr closed: the process is ending. Poll rather than wait() so
    // cancel() can still take the lock and kill it.
    let code = loop {
        let status = match child.lock().unwrap().as_mut() {
            Some(c) => c.try_wait(),
            None => break None,
        };
        match status {
            Ok(Some(s)) => break s.code(),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let _ = tx.send(Event::Exit(code));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        assert_eq!(
            parse_line(r#"{"event": "stage", "stage": "stems", "message": "separating"}"#),
            Event::Stage {
                stage: "stems".into(),
                message: "separating".into()
            }
        );
        assert_eq!(
            parse_line(r#"{"event": "done", "song_dir": "/x/y", "skipped": true}"#),
            Event::Done {
                song_dir: "/x/y".into(),
                skipped: true
            }
        );
        assert_eq!(
            parse_line(r#"{"event": "error", "message": "ffmpeg missing"}"#),
            Event::Error("ffmpeg missing".into())
        );
        assert_eq!(
            parse_line(r#"{"event": "lyrics", "status": "synced", "lines": 3, "language": "te"}"#),
            Event::Lyrics {
                status: "synced".into(),
                language: Some("te".into())
            }
        );
        assert_eq!(
            parse_line("UserWarning: x"),
            Event::Log("UserWarning: x".into())
        );
    }

    #[test]
    fn arguments() {
        let o = Options::default();
        assert_eq!(
            o.args(Path::new("/m/a.mp3")),
            [
                "/m/a.mp3",
                "--json-progress",
                "--mode",
                "chords+melody",
                "--simplify",
                "-d",
                "2"
            ]
        );
        let o = Options {
            mode: "simplified".into(),
            simplify: Some(4),
            no_separate: true,
            force: true,
        };
        assert_eq!(
            o.args(Path::new("a.wav"))[3..],
            ["simplified", "-d", "4", "--no-separate", "--force"]
        );
    }

    #[test]
    fn progress_only_moves_forward_through_stages() {
        let o = Options::default();
        let ex = expected_stages(&o, false);
        assert_eq!(ex.first(), Some(&"input"));
        assert!(ex.contains(&"stems") && ex.contains(&"melody"));
        let a = fraction(&ex, Some("decode"), 100.0);
        let b = fraction(&ex, Some("stems"), 0.0);
        let c = fraction(&ex, Some("stems"), 30.0);
        let d = fraction(&ex, Some("write"), 0.0);
        assert!(a <= b && b < c && c < d && d < 1.0, "{a} {b} {c} {d}");
        assert_eq!(fraction(&ex, None, 5.0), 0.0);
        let fast = expected_stages(
            &Options {
                no_separate: true,
                ..o
            },
            false,
        );
        assert!(!fast.contains(&"stems"));
    }

    /// Runs a fake analyzer script end to end (needs /bin/sh).
    #[test]
    fn runs_a_program_and_reads_its_events() {
        let dir = std::env::temp_dir().join(format!("qwertone-an-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-analyze");
        std::fs::write(
            &script,
            "#!/bin/sh\n\
             echo '{\"event\": \"stage\", \"stage\": \"input\", \"message\": \"'\"$1\"'\"}' >&2\n\
             echo 'some warning' >&2\n\
             echo '{\"event\": \"stage\", \"stage\": \"decode\", \"message\": \"decoding\"}' >&2\n\
             echo '{\"event\": \"done\", \"song_dir\": \"/tmp/song-x\"}' >&2\n\
             echo /tmp/song-x\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut job = Job::start(&script, Path::new("/music/a.mp3"), Options::default()).unwrap();
        let t = Instant::now();
        while job.running() && t.elapsed().as_secs() < 5 {
            job.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(job.result, Some(Ok(PathBuf::from("/tmp/song-x"))));
        assert_eq!(job.stage.as_deref(), Some("decode"));
        assert!(job.log.iter().any(|l| l == "some warning"));
        assert_eq!(job.fraction(), 1.0);

        // A failing run reports its last words.
        std::fs::write(&script, "#!/bin/sh\necho 'boom: no ffmpeg' >&2\nexit 3\n").unwrap();
        let mut job = Job::start(&script, Path::new("a.mp3"), Options::default()).unwrap();
        let t = Instant::now();
        while job.running() && t.elapsed().as_secs() < 5 {
            job.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let err = job.result.clone().unwrap().unwrap_err();
        assert!(
            err.contains("exit code 3") && err.contains("no ffmpeg"),
            "{err}"
        );

        // The lyrics command: no "done" event, a clean exit is success.
        std::fs::write(
            &script,
            "#!/bin/sh\n\
             [ \"$1\" = lyrics ] || exit 9\n\
             [ \"$4\" = --force ] || exit 8\n\
             echo '{\"event\": \"stage\", \"stage\": \"lyrics\", \"message\": \"transcribing 60 / 240 s\"}' >&2\n\
             echo '{\"event\": \"lyrics\", \"status\": \"not_found\", \"lines\": 0, \"language\": null}' >&2\n",
        )
        .unwrap();
        let mut job = Job::lyrics(&script, Path::new("/songs/x"), &["--force"]).unwrap();
        let t = Instant::now();
        while job.running() && t.elapsed().as_secs() < 5 {
            job.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(job.result, Some(Ok(PathBuf::from("/songs/x"))));
        assert_eq!(job.lyrics_status.as_deref(), Some("not_found"));
        assert_eq!(job.message, "transcribing 60 / 240 s");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
