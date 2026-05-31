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
    sessions: Mutex<HashMap<Uuid, TerminalSession>>,
    output_handler: TerminalOutputHandler,
}

struct TerminalSession {
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
}

impl TerminalService {
    pub fn new(output_handler: TerminalOutputHandler) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
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
        thread::spawn(move || read_pty_loop(session_id, &mut reader, handler));

        self.sessions.lock().map_err(lock_error)?.insert(
            info.id,
            TerminalSession {
                writer,
                child,
                _master: pair.master,
            },
        );

        Ok(info)
    }

    pub fn write(&self, session_id: Uuid, data: &str) -> AppResult<()> {
        let mut sessions = self.sessions.lock().map_err(lock_error)?;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| AppError::NotFound(format!("terminal session {session_id}")))?;
        session.writer.write_all(data.as_bytes())?;
        session.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, session_id: Uuid, cols: u16, rows: u16) -> AppResult<()> {
        let sessions = self.sessions.lock().map_err(lock_error)?;
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| AppError::NotFound(format!("terminal session {session_id}")))?;
        session
            ._master
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
        let mut sessions = self.sessions.lock().map_err(lock_error)?;
        sessions.remove(&session_id);
        Ok(())
    }

    pub fn close_all(&self) -> AppResult<()> {
        self.sessions.lock().map_err(lock_error)?.clear();
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn read_pty_loop(session_id: Uuid, reader: &mut dyn Read, handler: TerminalOutputHandler) {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let data = String::from_utf8_lossy(&buffer[..read]).to_string();
                handler(session_id, data);
            }
            Err(_) => break,
        }
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> AppError {
    AppError::Service("terminal service lock poisoned".to_string())
}
