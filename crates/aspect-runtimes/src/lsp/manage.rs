use tokio::sync::Mutex;

use super::LspInstallEvent;

pub(crate) static NPM_INSTALL_LOCK: Mutex<()> = Mutex::const_new(());
pub(crate) static GO_INSTALL_LOCK: Mutex<()> = Mutex::const_new(());
pub(crate) static PIP_INSTALL_LOCK: Mutex<()> = Mutex::const_new(());
pub(crate) static RUSTUP_INSTALL_LOCK: Mutex<()> = Mutex::const_new(());
pub(crate) static GH_INSTALL_LOCK: Mutex<()> = Mutex::const_new(());

/// Acquire a per-target install lock, surfacing a "waiting" progress step.
pub async fn acquire_install_lock<'lock>(
    _data_dir: &std::path::Path,
    language_id: &str,
    lock: &'lock Mutex<()>,
    command: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> tokio::sync::MutexGuard<'lock, ()> {
    if let Ok(guard) = lock.try_lock() {
        return guard;
    }
    on_event(LspInstallEvent::Progress {
        language_id: language_id.to_string(),
        percent: 15,
        step: format!("Waiting for another {command} install to finish"),
    });
    lock.lock().await
}
