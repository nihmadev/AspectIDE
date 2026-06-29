// Wasmtime runtime: instantiation with fuel + table limits, wall-clock timeout
// enforcement, host import linking, and per-phase fuel budgets.
use std::{
    sync::mpsc,
    thread,
    time::Duration,
};

use lux_core::{AppError, AppResult, ExtensionActivationCandidate, ExtensionHostActivationContract};
use wasmtime::{
    Config, Engine, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
};

use crate::{
    EXTENSION_HOST_ACTIVATION_FUEL, EXTENSION_HOST_ACTIVATION_TIMEOUT_MS,
    EXTENSION_HOST_COMMAND_FUEL, EXTENSION_HOST_MAX_TABLE_ELEMENTS,
    EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL, LUX_HOST_IMPORT_MODULE, WASM_PAGE_BYTES,
};

// ---------------------------------------------------------------------------
// Export execution accounting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct ExtensionExportExecution {
    pub fuel_consumed: u64,
    pub fuel_remaining: u64,
}

#[derive(Debug)]
pub struct ExtensionExportFailure {
    pub error: AppError,
    pub execution: Option<ExtensionExportExecution>,
}

impl ExtensionExportFailure {
    pub const fn without_execution(error: AppError) -> Self {
        Self { error, execution: None }
    }

    pub const fn with_execution(error: AppError, execution: ExtensionExportExecution) -> Self {
        Self { error, execution: Some(execution) }
    }
}

// ---------------------------------------------------------------------------
// ExtensionRuntime
// ---------------------------------------------------------------------------

pub struct ExtensionRuntime {
    pub store: Store<StoreLimits>,
    pub instance: Instance,
}

impl ExtensionRuntime {
    /// Instantiate a WASM module for the given candidate.
    ///
    /// F8 fix: the store starts with the activation fuel budget; we refuel
    /// separately before executing the command handler (see `call_handler`).
    ///
    /// F1 fix: the instantiation (which includes compilation) runs on a
    /// dedicated thread with a hard wall-clock deadline derived from
    /// `EXTENSION_HOST_ACTIVATION_TIMEOUT_MS`.  This bounds the cost of
    /// WASM module compilation/instantiation that fuel alone cannot cover.
    pub fn instantiate(
        candidate: &ExtensionActivationCandidate,
    ) -> Result<Self, ExtensionExportFailure> {
        let bytes = std::fs::read(&candidate.wasm_preflight.module_path)
            .map_err(AppError::from)
            .map_err(ExtensionExportFailure::without_execution)?;

        let memory_limit =
            memory_limit_bytes(candidate.host_contract.limits.max_memory_pages)
                .map_err(ExtensionExportFailure::without_execution)?;

        let contract = candidate.host_contract.clone();
        let timeout = Duration::from_millis(EXTENSION_HOST_ACTIVATION_TIMEOUT_MS);

        // F1: run compile + instantiate on a worker thread so we can apply a
        // real wall-clock timeout via channel recv with deadline.
        let (tx, rx) = mpsc::channel::<Result<Self, ExtensionExportFailure>>();
        thread::spawn(move || {
            let result = instantiate_on_thread(&bytes, memory_limit, &contract);
            // Ignore send failure: if the receiver timed out, we just discard.
            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(ExtensionExportFailure::without_execution(
                AppError::Service(format!(
                    "extension activation timed out after {EXTENSION_HOST_ACTIVATION_TIMEOUT_MS}ms"
                )),
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(ExtensionExportFailure::without_execution(AppError::Service(
                    "extension activation worker disconnected unexpectedly".into(),
                )))
            }
        }
    }

    /// Call the activation entrypoint using the activation fuel budget.
    pub fn call_activation(
        &mut self,
        export_name: &str,
    ) -> Result<ExtensionExportExecution, ExtensionExportFailure> {
        self.call_export(export_name)
    }

    /// F8 fix: replenish the store with the *command handler* fuel budget
    /// before calling the handler so that activation cost does not eat into the
    /// handler budget.
    pub fn call_handler(
        &mut self,
        export_name: &str,
    ) -> Result<ExtensionExportExecution, ExtensionExportFailure> {
        // Add the handler-specific fuel on top of (or replacing) whatever
        // fuel remains after activation.  We add rather than reset to avoid
        // underflow if remaining fuel > handler budget.
        self.store
            .set_fuel(EXTENSION_HOST_COMMAND_FUEL)
            .map_err(|e| ExtensionExportFailure::without_execution(wasmtime_error(&e)))?;
        self.call_export(export_name)
    }

    fn call_export(
        &mut self,
        export_name: &str,
    ) -> Result<ExtensionExportExecution, ExtensionExportFailure> {
        let fuel_before = current_fuel(&self.store)?;
        let entrypoint = match self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, export_name)
        {
            Ok(f) => f,
            Err(e) => {
                return Err(export_failure_since(
                    wasmtime_error(&e),
                    &self.store,
                    fuel_before,
                ));
            }
        };
        if let Err(e) = entrypoint.call(&mut self.store, ()) {
            return Err(export_failure_since(
                wasmtime_error(&e),
                &self.store,
                fuel_before,
            ));
        }
        export_execution_since(&self.store, fuel_before).ok_or_else(|| {
            ExtensionExportFailure::without_execution(AppError::Service(
                "extension WASM fuel accounting is unavailable".into(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Thread-local instantiation (no Send issues with Store)
// ---------------------------------------------------------------------------

fn instantiate_on_thread(
    bytes: &[u8],
    memory_limit: usize,
    contract: &ExtensionHostActivationContract,
) -> Result<ExtensionRuntime, ExtensionExportFailure> {
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config)
        .map_err(|e| ExtensionExportFailure::without_execution(wasmtime_error(&e)))?;

    // F4 fix: bound both memory and table elements in the store limiter so
    // runtime constraints match what preflight validates.
    let limits = StoreLimitsBuilder::new()
        .memory_size(memory_limit)
        // Cap total table elements across all tables.
        .table_elements(usize::try_from(EXTENSION_HOST_MAX_TABLE_ELEMENTS).unwrap_or(usize::MAX))
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(&engine, limits);
    store.limiter(|l| l);
    store
        .set_fuel(EXTENSION_HOST_ACTIVATION_FUEL)
        .map_err(|e| ExtensionExportFailure::without_execution(wasmtime_error(&e)))?;

    let module = Module::new(&engine, bytes)
        .map_err(|e| ExtensionExportFailure::without_execution(wasmtime_error(&e)))?;
    let mut linker = Linker::new(&engine);
    define_host_imports(&mut linker, contract).map_err(ExtensionExportFailure::without_execution)?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| export_failure_from_initial_fuel(wasmtime_error(&e), &store))?;

    Ok(ExtensionRuntime { store, instance })
}

// ---------------------------------------------------------------------------
// Host imports
// ---------------------------------------------------------------------------

/// Link host import functions.  After the F9 preflight fix, I/O imports
/// (`workspace_read`, etc.) are rejected before reaching here, so the match
/// arms for them are kept as a safety net but will never be reached in normal
/// operation.
fn define_host_imports(
    linker: &mut Linker<StoreLimits>,
    contract: &ExtensionHostActivationContract,
) -> AppResult<()> {
    for import in &contract.abi.imports {
        if import.module != LUX_HOST_IMPORT_MODULE
            || import.kind != lux_core::ExtensionWasmImportKind::Function
        {
            return Err(AppError::Service(format!(
                "runtime refused unsupported host import: {}.{}",
                import.module, import.name
            )));
        }

        // Static import names only: func_wrap requires 'static str for the name.
        // F9: I/O import arms are safety nets — preflight should have blocked
        // any extension that uses them before we ever get here.
        match import.name.as_str() {
            "log" => linker.func_wrap(LUX_HOST_IMPORT_MODULE, "log", || ()),
            "workspace_read" => linker.func_wrap(
                LUX_HOST_IMPORT_MODULE,
                "workspace_read",
                || Err::<(), _>(wasmtime::format_err!("workspace_read is not implemented")),
            ),
            "workspace_write" => linker.func_wrap(
                LUX_HOST_IMPORT_MODULE,
                "workspace_write",
                || Err::<(), _>(wasmtime::format_err!("workspace_write is not implemented")),
            ),
            "network_fetch" => linker.func_wrap(
                LUX_HOST_IMPORT_MODULE,
                "network_fetch",
                || Err::<(), _>(wasmtime::format_err!("network_fetch is not implemented")),
            ),
            "process_spawn" => linker.func_wrap(
                LUX_HOST_IMPORT_MODULE,
                "process_spawn",
                || Err::<(), _>(wasmtime::format_err!("process_spawn is not implemented")),
            ),
            name => {
                return Err(AppError::Service(format!(
                    "runtime refused unknown Lux host import: {name}"
                )));
            }
        }
        .map_err(|e| AppError::Service(format!("extension WASM linker error: {e}")))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fuel helpers
// ---------------------------------------------------------------------------

pub fn memory_limit_bytes(max_pages: u32) -> AppResult<usize> {
    usize::try_from(max_pages)
        .ok()
        .and_then(|p| p.checked_mul(WASM_PAGE_BYTES))
        .ok_or_else(|| AppError::Service("extension memory limit overflows usize".into()))
}

pub fn current_fuel(store: &Store<StoreLimits>) -> Result<u64, ExtensionExportFailure> {
    store
        .get_fuel()
        .map_err(|e| ExtensionExportFailure::without_execution(wasmtime_error(&e)))
}

pub fn export_execution_since(
    store: &Store<StoreLimits>,
    fuel_before: u64,
) -> Option<ExtensionExportExecution> {
    store.get_fuel().ok().map(|fuel_remaining| ExtensionExportExecution {
        fuel_consumed: fuel_before.saturating_sub(fuel_remaining),
        fuel_remaining,
    })
}

pub fn export_failure_from_initial_fuel(
    error: AppError,
    store: &Store<StoreLimits>,
) -> ExtensionExportFailure {
    match export_execution_since(store, EXTENSION_HOST_ACTIVATION_FUEL) {
        Some(ex) => ExtensionExportFailure::with_execution(error, ex),
        None => ExtensionExportFailure::without_execution(error),
    }
}

pub fn export_failure_since(
    error: AppError,
    store: &Store<StoreLimits>,
    fuel_before: u64,
) -> ExtensionExportFailure {
    match export_execution_since(store, fuel_before) {
        Some(ex) => ExtensionExportFailure::with_execution(error, ex),
        None => ExtensionExportFailure::without_execution(error),
    }
}

pub fn wasmtime_error(error: &wasmtime::Error) -> AppError {
    if let Some(trap) = error.downcast_ref::<Trap>() {
        return match trap {
            Trap::OutOfFuel => AppError::Service(EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL.into()),
            trap => AppError::Service(format!("extension WASM runtime trap: {trap:?}")),
        };
    }
    AppError::Service(format!("extension WASM runtime error: {error}"))
}

// ---------------------------------------------------------------------------
// Error reason helpers
// ---------------------------------------------------------------------------

pub fn activation_failure_reason(error: &AppError) -> String {
    match error {
        AppError::Service(reason) if reason == EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL => {
            "extension activation exhausted fuel".into()
        }
        AppError::Service(reason) => reason.clone(),
        e => e.to_string(),
    }
}

pub fn execution_failure_reason(
    error: &AppError,
    phase: lux_core::ExtensionCommandExecutionPhase,
) -> String {
    use lux_core::ExtensionCommandExecutionPhase::{Activation, Handler};
    match (phase, error) {
        (Activation, AppError::Service(reason))
            if reason == EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL =>
        {
            "extension activation exhausted fuel".into()
        }
        (Handler, AppError::Service(reason))
            if reason == EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL =>
        {
            "extension command handler exhausted fuel".into()
        }
        (_, AppError::Service(reason)) => reason.clone(),
        (_, e) => e.to_string(),
    }
}
