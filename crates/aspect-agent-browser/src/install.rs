use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tauri::Manager;

use crate::resolver::{
    agent_browser_command, desktop_package_dir, managed_install_dir, resolve_binary,
    INSTALL_TIMEOUT_SECS,
};
use crate::types::{
    AgentBrowserInstallRequest, AgentBrowserInstallResponse, AgentBrowserInstallStep,
};

fn resolve_host_tool(app: &tauri::AppHandle, tool: &str) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    for runtime_dir in aspect_runtimes::runtime::runtime_bin_dirs(&dir) {
        if let Some(path) = aspect_runtimes::resolve::resolve_in_dir(&runtime_dir, tool) {
            return Some(path);
        }
    }
    aspect_runtimes::resolve::resolve_on_path(tool)
}

async fn resolve_install_plan(
    app: &tauri::AppHandle,
    steps: &mut Vec<AgentBrowserInstallStep>,
) -> Result<(PathBuf, Vec<String>, Option<PathBuf>), String> {
    if let Some(desktop_dir) = desktop_package_dir() {
        if let Some(pnpm) = aspect_runtimes::resolve::resolve_on_path("pnpm") {
            return Ok((
                pnpm,
                vec!["add".to_string(), "agent-browser@latest".to_string()],
                Some(desktop_dir),
            ));
        }
        if let Some(npm) = resolve_host_tool(app, "npm") {
            return Ok((
                npm,
                vec!["install".to_string(), "agent-browser@latest".to_string()],
                Some(desktop_dir),
            ));
        }
    }

    let npm = if let Some(npm) = resolve_host_tool(app, "npm") {
        npm
    } else {
        let started = Instant::now();
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let discard = |_: aspect_runtimes::runtime::RuntimeProvisionEvent| {};
        let result =
            aspect_runtimes::runtime::ensure_runtime(&dir, aspect_runtimes::runtime::Runtime::Node, &discard)
                .await;
        steps.push(AgentBrowserInstallStep {
            name: "node-runtime-setup".to_string(),
            success: result.is_ok(),
            output: match &result {
                Ok(path) => format!("Managed Node.js ready at {path}"),
                Err(error) => error.clone(),
            },
            elapsed_ms: started.elapsed().as_millis(),
        });
        result.map_err(|error| {
            format!("agent-browser needs Node.js and automatic Node setup failed: {error}")
        })?;
        resolve_host_tool(app, "npm")
            .ok_or_else(|| "Node.js was set up but npm is still not resolvable.".to_string())?
    };

    let prefix = managed_install_dir().ok_or_else(|| {
        "App data directory is not available for the managed install.".to_string()
    })?;
    tokio::fs::create_dir_all(&prefix)
        .await
        .map_err(|error| format!("Could not create {}: {error}", prefix.display()))?;
    Ok((
        npm,
        vec![
            "install".to_string(),
            "--prefix".to_string(),
            prefix.to_string_lossy().to_string(),
            "--no-audit".to_string(),
            "--no-fund".to_string(),
            "--loglevel".to_string(),
            "error".to_string(),
            "agent-browser@latest".to_string(),
        ],
        None,
    ))
}

pub async fn install(
    app: &tauri::AppHandle,
    request: AgentBrowserInstallRequest,
) -> Result<AgentBrowserInstallResponse, String> {
    let mut steps = Vec::new();
    let (package_manager, install_args, working_dir) =
        resolve_install_plan(app, &mut steps).await?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let env: Vec<(String, String)> = aspect_runtimes::runtime::prepended_path(&dir)
        .into_iter()
        .collect();
    let local_install = run_install_step_in_dir(
        "package-install-latest",
        package_manager,
        install_args,
        working_dir.as_deref(),
        &env,
        INSTALL_TIMEOUT_SECS,
    )
    .await;
    steps.push(local_install);

    let binary = resolve_binary().ok();

    let chrome_args = if request.with_deps == Some(true) {
        vec!["install".to_string(), "--with-deps".to_string()]
    } else {
        vec!["install".to_string()]
    };

    if let Some(ref binary) = binary {
        let chrome_step = run_install_step(
            "agent-browser-install-chrome",
            binary.clone(),
            chrome_args,
            INSTALL_TIMEOUT_SECS,
        )
        .await;
        steps.push(chrome_step);
    }

    let command_path = resolve_binary().ok();
    let success = steps.iter().all(|step| step.success) && command_path.is_some();
    let detail = if success {
        "agent-browser installed successfully.".to_string()
    } else if steps.iter().all(|step| step.success) {
        "Install steps finished but the agent-browser CLI is still not resolvable. \
         Re-run the install or check Settings -> Browser automation."
            .to_string()
    } else {
        install_failure_detail(&steps)
    };

    Ok(AgentBrowserInstallResponse {
        success,
        command_path: command_path.map(|path| path.display().to_string()),
        steps,
        detail,
    })
}

const FAILURE_DETAIL_OUTPUT_CAP: usize = 800;

fn install_failure_detail(steps: &[AgentBrowserInstallStep]) -> String {
    let Some(failed) = steps.iter().find(|step| !step.success) else {
        return "agent-browser installation finished with errors. Review step output.".to_string();
    };

    let captured = failed.output.trim();
    let output_hint = if captured.is_empty() {
        " No output was captured for this step; re-run the install from a terminal \
         to see the underlying error."
            .to_string()
    } else {
        let bounded = bounded_tail(captured, FAILURE_DETAIL_OUTPUT_CAP);
        format!(" Output from '{}':\n{bounded}", failed.name)
    };

    format!(
        "agent-browser installation failed at step '{}'.{output_hint}",
        failed.name
    )
}

fn bounded_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max_chars).collect();
    format!(
        "[...{} earlier characters omitted...]\n{tail}",
        count - max_chars
    )
}

async fn run_install_step_in_dir(
    name: &str,
    program: PathBuf,
    args: Vec<String>,
    working_dir: Option<&Path>,
    env: &[(String, String)],
    timeout_secs: u64,
) -> AgentBrowserInstallStep {
    let started = Instant::now();
    let mut command = agent_browser_command(&program);
    command.args(&args);
    for (key, value) in env {
        command.env(key, value);
    }
    if let Some(dir) = working_dir {
        command.current_dir(dir);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), command.output()).await;
    match output {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}").trim().to_string();
            AgentBrowserInstallStep {
                name: name.to_string(),
                success: output.status.success(),
                output: combined,
                elapsed_ms: started.elapsed().as_millis(),
            }
        }
        Ok(Err(error)) => AgentBrowserInstallStep {
            name: name.to_string(),
            success: false,
            output: format!("Failed to start install step: {error}"),
            elapsed_ms: started.elapsed().as_millis(),
        },
        Err(_) => AgentBrowserInstallStep {
            name: name.to_string(),
            success: false,
            output: format!("Install step timed out after {timeout_secs}s"),
            elapsed_ms: started.elapsed().as_millis(),
        },
    }
}

async fn run_install_step(
    name: &str,
    program: PathBuf,
    args: Vec<String>,
    timeout_secs: u64,
) -> AgentBrowserInstallStep {
    run_install_step_in_dir(name, program, args, None, &[], timeout_secs).await
}
