// WASM binary pre-flight validation: path safety, magic/size checks, section
// scanning for ABI contract, memory + table limits, import policy.
use std::{fs, io::Read};

use lux_core::{
    AppError, AppResult, ExtensionHostActivationContract, ExtensionHostLimits, ExtensionInfo,
    ExtensionWasmAbi, ExtensionWasmImport, ExtensionWasmImportKind, ExtensionWasmPreflight,
};
use wasmparser::{
    CompositeInnerType, Encoding, ExternalKind, Parser, Payload, TypeRef, Validator,
};

use crate::{
    ALLOWED_HOST_IMPORTS, EXTENSION_HOST_ACTIVATION_TIMEOUT_MS, EXTENSION_HOST_MAX_MEMORY_PAGES,
    EXTENSION_HOST_MAX_OUTPUT_BYTES, EXTENSION_HOST_MAX_TABLE_ELEMENTS,
    LUX_EXTENSION_ABI_VERSION, LUX_EXTENSION_ENTRYPOINT, LUX_EXTENSION_OPTIONAL_EXPORTS,
    LUX_HOST_IMPORT_MODULE, MAX_WASM_MODULE_BYTES, WASM_MAGIC_AND_VERSION,
};

/// Parameter / result counts of a defined WASM function type, used by the F3
/// host-import signature check.
#[derive(Debug, Clone, Copy)]
struct FuncArity {
    params: usize,
    results: usize,
}

pub fn validate_wasm_preflight(extension: &ExtensionInfo) -> AppResult<ExtensionWasmPreflight> {
    let root = extension.root.canonicalize()?;
    let module_path = extension.wasm_module.canonicalize()?;
    if !module_path.starts_with(&root) {
        return Err(AppError::Service(format!(
            "WASM module escapes extension root: {}",
            module_path.display()
        )));
    }

    let metadata = fs::metadata(&module_path)?;
    if !metadata.is_file() {
        return Err(AppError::Service(format!(
            "WASM module is not a file: {}",
            module_path.display()
        )));
    }
    if metadata.len() > MAX_WASM_MODULE_BYTES {
        return Err(AppError::Service(format!(
            "WASM module is too large: {} bytes > {MAX_WASM_MODULE_BYTES}",
            metadata.len()
        )));
    }

    let mut header = [0_u8; 8];
    let mut file = fs::File::open(&module_path)?;
    file.read_exact(&mut header)?;
    if header != WASM_MAGIC_AND_VERSION {
        return Err(AppError::Service(format!(
            "WASM module has invalid magic or version: {}",
            module_path.display()
        )));
    }

    Ok(ExtensionWasmPreflight {
        module_path,
        size_bytes: metadata.len(),
    })
}

pub fn validate_wasm_host_contract(
    extension: &ExtensionInfo,
    preflight: &ExtensionWasmPreflight,
) -> AppResult<ExtensionHostActivationContract> {
    let bytes = fs::read(&preflight.module_path)?;
    Validator::new()
        .validate_all(&bytes)
        .map_err(|e| AppError::Service(e.to_string()))?;

    let mut exported_entrypoint = false;
    let mut exports_memory = false;
    let mut imports = Vec::new();
    let mut exported_functions = Vec::new();
    // F3 fix: collect each defined function type's (param_count, result_count)
    // in type-index order so we can validate that host function imports match
    // the exact ABI signature (zero params, zero results) the host links,
    // moving the failure from activation time to deterministic preflight.
    // `None` marks a non-function type so indices stay aligned with the WASM
    // type index space.
    let mut func_type_arity: Vec<Option<FuncArity>> = Vec::new();

    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.map_err(|e| AppError::Service(e.to_string()))? {
            Payload::Version { encoding, .. } => {
                if encoding != Encoding::Module {
                    return Err(AppError::Service(
                        "extension WASM must be a core module, not a component".into(),
                    ));
                }
            }
            Payload::TypeSection(reader) => {
                collect_func_type_arity(reader, &mut func_type_arity)?;
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(|e| AppError::Service(e.to_string()))?;
                    validate_host_import(
                        extension,
                        import.module,
                        import.name,
                        import.ty,
                        &func_type_arity,
                    )?;
                    imports.push(ExtensionWasmImport {
                        module: import.module.to_string(),
                        name: import.name.to_string(),
                        kind: import_kind(import.ty),
                    });
                }
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory.map_err(|e| AppError::Service(e.to_string()))?;
                    validate_memory_limit(memory.initial, memory.maximum)?;
                }
            }
            // F4 fix: validate table section so a malicious extension cannot
            // exhaust host resources through large table declarations.
            Payload::TableSection(reader) => {
                for table in reader {
                    let table = table.map_err(|e| AppError::Service(e.to_string()))?;
                    validate_table_limit(table.ty.initial, table.ty.maximum)?;
                }
            }
            Payload::ExportSection(reader) => {
                process_export_section(
                    reader,
                    &mut exported_entrypoint,
                    &mut exports_memory,
                    &mut exported_functions,
                )?;
            }
            _ => {}
        }
    }

    if !exported_entrypoint {
        return Err(AppError::Service(format!(
            "WASM module must export required Lux extension entrypoint: {LUX_EXTENSION_ENTRYPOINT}"
        )));
    }
    validate_command_handler_exports(extension, &exported_functions)?;

    imports.sort_by(|l, r| l.module.cmp(&r.module).then_with(|| l.name.cmp(&r.name)));

    Ok(ExtensionHostActivationContract {
        abi: ExtensionWasmAbi {
            version: LUX_EXTENSION_ABI_VERSION,
            entrypoint: LUX_EXTENSION_ENTRYPOINT.to_string(),
            required_exports: vec![LUX_EXTENSION_ENTRYPOINT.to_string()],
            optional_exports: LUX_EXTENSION_OPTIONAL_EXPORTS
                .iter()
                .map(|v| (*v).to_string())
                .collect(),
            imports,
            exports_memory,
        },
        permissions: extension.permissions.clone(),
        limits: ExtensionHostLimits {
            max_memory_pages: EXTENSION_HOST_MAX_MEMORY_PAGES,
            activation_timeout_ms: EXTENSION_HOST_ACTIVATION_TIMEOUT_MS,
            max_output_bytes: EXTENSION_HOST_MAX_OUTPUT_BYTES,
        },
    })
}

/// F3 helper: record each defined function type's arity in type-index order.
/// Non-function types push `None` so indices stay aligned with the WASM type
/// index space, letting host-import signatures be resolved by index.
fn collect_func_type_arity(
    reader: wasmparser::TypeSectionReader,
    func_type_arity: &mut Vec<Option<FuncArity>>,
) -> AppResult<()> {
    for rec_group in reader {
        let rec_group = rec_group.map_err(|e| AppError::Service(e.to_string()))?;
        for sub_type in rec_group.types() {
            func_type_arity.push(match &sub_type.composite_type.inner {
                CompositeInnerType::Func(func) => Some(FuncArity {
                    params: func.params().len(),
                    results: func.results().len(),
                }),
                _ => None,
            });
        }
    }
    Ok(())
}

fn process_export_section(
    reader: wasmparser::ExportSectionReader,
    exported_entrypoint: &mut bool,
    exports_memory: &mut bool,
    exported_functions: &mut Vec<String>,
) -> AppResult<()> {
    for export in reader {
        let export = export.map_err(|e| AppError::Service(e.to_string()))?;
        if export.name == LUX_EXTENSION_ENTRYPOINT {
            if export.kind != ExternalKind::Func {
                return Err(AppError::Service(format!(
                    "required export {LUX_EXTENSION_ENTRYPOINT} must be a function"
                )));
            }
            *exported_entrypoint = true;
        }
        if export.kind == ExternalKind::Func {
            exported_functions.push(export.name.to_string());
        }
        if export.name == "memory" {
            if export.kind != ExternalKind::Memory {
                return Err(AppError::Service(
                    "export named memory must be a WebAssembly memory".into(),
                ));
            }
            *exports_memory = true;
        }
    }
    Ok(())
}

fn validate_command_handler_exports(
    extension: &ExtensionInfo,
    exported_functions: &[String],
) -> AppResult<()> {
    for command in &extension.commands {
        if !exported_functions.iter().any(|e| e == &command.handler) {
            return Err(AppError::Service(format!(
                "extension command {} references missing WASM handler export: {}",
                command.id, command.handler
            )));
        }
    }
    Ok(())
}

/// F9 fix: permitted I/O imports (`workspace_read`/`workspace_write`,
/// `network_fetch`, `process_spawn`) are listed in the contract but the actual Wasmtime host
/// functions are not yet implemented — they would always trap. Rather than
/// silently lying to the extension, we reject them during preflight with a
/// clear "not-yet-implemented" reason so the extension author gets an honest
/// error at load time instead of a mysterious runtime trap.
fn validate_host_import(
    extension: &ExtensionInfo,
    module: &str,
    name: &str,
    ty: TypeRef,
    func_type_arity: &[Option<FuncArity>],
) -> AppResult<()> {
    if module != LUX_HOST_IMPORT_MODULE {
        return Err(AppError::Service(format!(
            "unsupported WASM import module: {module}.{name}"
        )));
    }
    let (TypeRef::Func(type_index) | TypeRef::FuncExact(type_index)) = ty else {
        return Err(AppError::Service(format!(
            "Lux host import must be a function: {module}.{name}"
        )));
    };

    // F3 fix: validate the exact ABI signature.  Every linked Lux host function
    // is zero-argument / zero-result (`func_wrap(.., || ())`), so an import with
    // any other signature would only fail at activation.  Reject it here so the
    // mismatch surfaces as a deterministic preflight error instead.
    let arity = func_type_arity
        .get(type_index as usize)
        .and_then(|slot| slot.as_ref())
        .ok_or_else(|| {
            AppError::Service(format!(
                "Lux host import {module}.{name} references an undefined or non-function type \
                 (index {type_index})"
            ))
        })?;
    if arity.params != 0 || arity.results != 0 {
        return Err(AppError::Service(format!(
            "Lux host import {module}.{name} has unsupported signature \
             ({} params, {} results); host imports must take no arguments and return nothing",
            arity.params, arity.results
        )));
    }

    let Some(spec) = ALLOWED_HOST_IMPORTS
        .iter()
        .find(|candidate| candidate.name == name)
    else {
        return Err(AppError::Service(format!(
            "unsupported Lux host import: {module}.{name}"
        )));
    };

    if let Some(permission) = spec.permission {
        if !extension.permissions.contains(&permission) {
            return Err(AppError::Service(format!(
                "WASM import {module}.{name} requires manifest permission {permission:?}"
            )));
        }
    }

    // F9: reject I/O imports that are permitted by the contract but have no
    // real host-side implementation (they would always trap at runtime).
    if spec.permission.is_some() {
        return Err(AppError::Service(format!(
            "Lux host import '{name}' is declared in the contract but is not yet implemented; \
             extension must not import it until the host provides a real implementation"
        )));
    }

    Ok(())
}

fn validate_memory_limit(initial: u64, maximum: Option<u64>) -> AppResult<()> {
    let limit = u64::from(EXTENSION_HOST_MAX_MEMORY_PAGES);
    if initial > limit {
        return Err(AppError::Service(format!(
            "WASM memory initial pages exceed host limit: {initial} > {EXTENSION_HOST_MAX_MEMORY_PAGES}"
        )));
    }
    if let Some(maximum) = maximum {
        if maximum > limit {
            return Err(AppError::Service(format!(
                "WASM memory maximum pages exceed host limit: {maximum} > {EXTENSION_HOST_MAX_MEMORY_PAGES}"
            )));
        }
    }
    Ok(())
}

/// F4 fix: bound table element counts in preflight so they are consistent with
/// `StoreLimitsBuilder::table_elements` enforced at runtime.
/// wasmparser reports table sizes as `u64` (matching the WASM spec); we
/// compare against the host limit cast to `u64` for correct comparison.
fn validate_table_limit(initial: u64, maximum: Option<u64>) -> AppResult<()> {
    let limit = u64::from(EXTENSION_HOST_MAX_TABLE_ELEMENTS);
    if initial > limit {
        return Err(AppError::Service(format!(
            "WASM table initial elements exceed host limit: {initial} > {EXTENSION_HOST_MAX_TABLE_ELEMENTS}"
        )));
    }
    if let Some(maximum) = maximum {
        if maximum > limit {
            return Err(AppError::Service(format!(
                "WASM table maximum elements exceed host limit: {maximum} > {EXTENSION_HOST_MAX_TABLE_ELEMENTS}"
            )));
        }
    }
    Ok(())
}

const fn import_kind(ty: TypeRef) -> ExtensionWasmImportKind {
    match ty {
        TypeRef::Func(_) | TypeRef::FuncExact(_) => ExtensionWasmImportKind::Function,
        TypeRef::Table(_) => ExtensionWasmImportKind::Table,
        TypeRef::Memory(_) => ExtensionWasmImportKind::Memory,
        TypeRef::Global(_) => ExtensionWasmImportKind::Global,
        TypeRef::Tag(_) => ExtensionWasmImportKind::Tag,
    }
}
