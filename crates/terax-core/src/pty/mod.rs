
use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::{io::{Read, Write}, path::PathBuf, sync::mpsc::{self, Receiver}, thread};

pub struct PtySession {
    writer: Box<dyn Write + Send>,
    rx: Receiver<String>,
    #[allow(dead_code)]
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtySession {
    pub fn spawn(shell: Option<String>, cols: u16, rows: u16, cwd: Option<PathBuf>) -> Result<Self> {
        let shell = shell.unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "sh".into()));
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::new(shell);
        if let Some(cwd) = cwd { cmd.cwd(cwd); }
        let child = pair.slave.spawn_command(cmd).context("spawn shell in PTY")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().context("clone PTY reader")?;
        let writer = pair.master.take_writer().context("take PTY writer")?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let s = String::from_utf8_lossy(&buf[..n]).to_string();
                        if tx.send(s).is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { writer, rx, child })
    }

    pub fn write(&mut self, s: &str) -> Result<()> {
        self.writer.write_all(s.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn try_read_all(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(s) = self.rx.try_recv() { out.push(s); }
        out
    }
}
