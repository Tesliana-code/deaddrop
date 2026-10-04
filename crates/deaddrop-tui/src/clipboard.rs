//! Reading the clipboard, for right-click paste in the composer.
//!
//! Windows Terminal pastes on Ctrl+V as a bracketed paste, but a right click
//! while the app has the mouse is only a mouse event. So a right click in
//! the composer reads the clipboard here. In WSL the Windows clipboard is
//! reached through `powershell.exe`, run with fixed arguments: the clipboard
//! comes back as output data and is never part of a command. Its contents
//! are not logged or kept.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub trait Clipboard {
    /// The clipboard's text, `None` when it holds no text.
    fn read_text(&self) -> Result<Option<String>, String>;
}

/// The Windows clipboard from WSL, through `powershell.exe`.
#[derive(Debug, Clone)]
pub struct WindowsClipboard {
    pub timeout: Duration,
}

impl Default for WindowsClipboard {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3),
        }
    }
}

/// Output UTF-8, write the text with nothing added, write nothing for an
/// empty or non-text clipboard. A fixed script: no input reaches it.
const SCRIPT: &str = "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
                      $t = Get-Clipboard -Raw; if ($t) { [Console]::Out.Write($t) }";

impl Clipboard for WindowsClipboard {
    fn read_text(&self) -> Result<Option<String>, String> {
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no clipboard bridge ({e})"))?;
        let mut stdout = child.stdout.take().expect("piped");
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        });
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if started.elapsed() >= self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err("clipboard read timed out".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let bytes = reader.join().unwrap_or_default();
        if !status.success() {
            return Err("clipboard read failed".into());
        }
        let text =
            String::from_utf8(bytes).map_err(|_| "clipboard is not UTF-8 text".to_owned())?;
        Ok((!text.is_empty()).then_some(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_is_fixed_and_takes_no_input() {
        assert!(SCRIPT.contains("Get-Clipboard -Raw"));
        assert!(SCRIPT.contains("UTF8"));
        assert!(!SCRIPT.contains("Set-Clipboard"), "read only");
    }
}
