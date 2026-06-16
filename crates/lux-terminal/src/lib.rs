#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

use chrono::Utc;
use lux_core::{AppError, AppResult, TerminalSessionInfo};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use uuid::Uuid;

pub type TerminalOutputHandler = Arc<dyn Fn(Uuid, String) + Send + Sync + 'static>;

#[must_use]
pub fn default_shell() -> String {
    if cfg!(target_os = "windows") {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

pub fn session_info(shell: Option<String>, cwd: PathBuf) -> TerminalSessionInfo {
    TerminalSessionInfo {
        id: Uuid::new_v4(),
        shell: shell.unwrap_or_else(default_shell),
        cwd,
        created_at: Utc::now(),
    }
}

pub struct TerminalService {
    sessions: Arc<Mutex<HashMap<Uuid, Arc<TerminalSession>>>>,
    output_handler: TerminalOutputHandler,
}

struct TerminalSession {
    writer: Mutex<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    master: Mutex<Box<dyn MasterPty + Send>>,
}

impl TerminalService {
    pub fn new(output_handler: TerminalOutputHandler) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            output_handler,
        }
    }

    pub fn create(
        &self,
        shell: Option<String>,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> AppResult<TerminalSessionInfo> {
        let info = session_info(shell, cwd);
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| AppError::Service(error.to_string()))?;

        let mut command = CommandBuilder::new(&info.shell);
        command.cwd(&info.cwd);

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| AppError::Service(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| AppError::Service(error.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| AppError::Service(error.to_string()))?;

        let session_id = info.id;
        let handler = Arc::clone(&self.output_handler);
        let sessions = Arc::clone(&self.sessions);
        thread::spawn(move || {
            read_pty_loop(session_id, &mut reader, &handler);
            // Shell exited on its own (e.g. user typed `exit`): drop the session so
            // its PTY handles close and the dead child is reaped via Drop. Take the
            // entry out under the lock, then drop it after releasing the lock.
            let removed = sessions.lock().ok().and_then(|mut s| s.remove(&session_id));
            drop(removed);
        });

        self.sessions.lock().map_err(lock_error)?.insert(
            info.id,
            Arc::new(TerminalSession {
                writer: Mutex::new(writer),
                child,
                master: Mutex::new(pair.master),
            }),
        );

        Ok(info)
    }

    pub fn write(&self, session_id: Uuid, data: &str) -> AppResult<()> {
        let session = self.session(session_id)?;
        let mut writer = session.writer.lock().map_err(lock_error)?;
        writer.write_all(data.as_bytes())?;
        writer.flush()?;
        drop(writer);
        Ok(())
    }

    pub fn resize(&self, session_id: Uuid, cols: u16, rows: u16) -> AppResult<()> {
        let session = self.session(session_id)?;
        session
            .master
            .lock()
            .map_err(lock_error)?
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| AppError::Service(error.to_string()))?;
        Ok(())
    }

    pub fn close(&self, session_id: Uuid) -> AppResult<()> {
        // Drop the removed session AFTER releasing the lock: TerminalSession::drop
        // runs a blocking child.wait(), which must not execute while the sessions
        // Mutex is held or a slow-dying child would hang every other terminal op.
        let removed = self
            .sessions
            .lock()
            .map_err(lock_error)?
            .remove(&session_id);
        drop(removed);
        Ok(())
    }

    pub fn close_all(&self) -> AppResult<()> {
        // Drain under the lock, then drop the sessions after releasing it so the
        // blocking child.wait() in TerminalSession::drop never runs while holding
        // the sessions Mutex (.clear() would drop every child serially under it).
        let drained: Vec<_> = {
            let mut guard = self.sessions.lock().map_err(lock_error)?;
            guard.drain().map(|(_, session)| session).collect()
        };
        drop(drained);
        Ok(())
    }

    fn session(&self, session_id: Uuid) -> AppResult<Arc<TerminalSession>> {
        self.sessions
            .lock()
            .map_err(lock_error)?
            .get(&session_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("terminal session {session_id}")))
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // Reap the child so a SIGKILL-fallback exit does not leave a zombie.
        let _ = self.child.wait();
    }
}

fn read_pty_loop(session_id: Uuid, reader: &mut dyn Read, handler: &TerminalOutputHandler) {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let data = String::from_utf8_lossy(&buffer[..read]).to_string();
                handler(session_id, data);
            }
        }
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> AppError {
    AppError::Service("terminal service lock poisoned".to_string())
}
