use std::{
    collections::{BTreeSet, VecDeque},
    path::{Component, Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;
use tokio::time::{timeout, Duration};

const AI_TEST_HEALTH_TIMEOUT_SECS: u64 = 180;
const AI_TEST_HEALTH_MAX_OUTPUT_CHARS: usize = 24_000;
const AI_TEST_HEALTH_SCAN_MAX_DEPTH: usize = 4;
const AI_TEST_HEALTH_MAX_RUNNERS: usize = 12;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const WATCH_EXCLUDED_COMPONENTS: &[&str] = &[
    ".git",
    ".next",
    ".turbo",
    ".vite",
    "coverage",
    "dist",
    "node_modules",
    "target",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestHealthResponse {
    workspace_root: PathBuf,
    status: String,
    summary: TestHealthSummary,
    runners: Vec<TestHealthRunnerResult>,
    language: String,
    framework: String,
    command: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestHealthSummary {
    total: usize,
    passed: usize,
    failed: usize,
    timed_out: usize,
    errored: usize,
    skipped: usize,
    duration_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestHealthRunnerResult {
    id: String,
    workspace_relative_path: String,
    status: String,
    kind: String,
    language: String,
    framework: String,
    command: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Clone)]
struct TestHealthPlan {
    kind: &'static str,
    language: &'static str,
    framework: &'static str,
    working_dir: PathBuf,
    command: String,
}

pub async fn run(root: PathBuf) -> Result<TestHealthResponse, String> {
    let plans = detect_test_health_plans(&root);
    run_test_health_plans(root, plans).await
}

fn detect_test_health_plans(root: &Path) -> Vec<TestHealthPlan> {
    let mut directories = collect_test_health_scan_dirs(root);
    directories.sort_by(|left, right| {
        relative_depth(root, left)
            .cmp(&relative_depth(root, right))
            .then_with(|| left.cmp(right))
    });

    let root_caps = RootTestHealthCapabilities::from_root(root);
    let mut plans = Vec::new();
    let mut seen = BTreeSet::new();
    for directory in directories {
        add_test_health_plans_for_dir(root, &directory, &root_caps, &mut seen, &mut plans);
    }
    plans
}

#[derive(Debug, Clone)]
struct RootTestHealthCapabilities {
    maven_multi_module: bool,
    gradle_multi_project: bool,
    dotnet: bool,
}

impl RootTestHealthCapabilities {
    fn from_root(root: &Path) -> Self {
        Self {
            maven_multi_module: maven_manifest_has_modules(root),
            gradle_multi_project: has_gradle_settings(root),
            dotnet: find_file_with_extension(root, "sln").is_some(),
        }
    }
}

fn add_test_health_plans_for_dir(
    root: &Path,
    directory: &Path,
    root_caps: &RootTestHealthCapabilities,
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
) {
    let is_root = same_path(root, directory);

    if has_package_test_script(directory) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "JavaScript/TypeScript",
            "package.json test script",
            directory,
            package_manager_script_command(directory, "test"),
        );
    } else {
        add_package_validation_plans(directory, seen, plans);
    }

    if directory.join("Cargo.toml").is_file() && !ancestor_is_cargo_workspace(root, directory) {
        let command = if cargo_manifest_has_workspace(directory) {
            "cargo test --workspace"
        } else {
            "cargo test"
        };
        push_test_health_command(seen, plans, "test", "Rust", "Cargo", directory, command);
    }

    if is_python_test_project(directory) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Python",
            "pytest",
            directory,
            python_test_command(directory),
        );
    }

    if directory.join("go.mod").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Go",
            "go test",
            directory,
            "go test ./...",
        );
    }

    if directory.join("pom.xml").is_file() && (is_root || !root_caps.maven_multi_module) {
        push_test_health_command(seen, plans, "test", "Java", "Maven", directory, "mvn test");
    }

    if is_gradle_project(directory) && (is_root || !root_caps.gradle_multi_project) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Java/Kotlin",
            "Gradle",
            directory,
            gradle_test_command(directory),
        );
    }

    if (find_file_with_extension(directory, "sln").is_some()
        || find_file_with_extension(directory, "csproj").is_some())
        && (is_root || !root_caps.dotnet)
    {
        push_test_health_command(
            seen,
            plans,
            "test",
            ".NET",
            "dotnet test",
            directory,
            dotnet_test_command(directory),
        );
    }

    if has_composer_test_script(directory) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "PHP",
            "Composer",
            directory,
            "composer test",
        );
    }

    if directory.join("Gemfile").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Ruby",
            "Bundler",
            directory,
            ruby_test_command(directory),
        );
    }

    if directory.join("mix.exs").is_file() {
        push_test_health_command(seen, plans, "test", "Elixir", "Mix", directory, "mix test");
    }

    if directory.join("Package.swift").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Swift",
            "SwiftPM",
            directory,
            "swift test",
        );
    }

    if directory.join("deno.json").is_file() || directory.join("deno.jsonc").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "TypeScript/JavaScript",
            "Deno",
            directory,
            "deno test",
        );
    }

    if directory.join("pubspec.yaml").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Dart/Flutter",
            "Dart test",
            directory,
            dart_test_command(directory),
        );
    }

    if directory.join("build.sbt").is_file() {
        push_test_health_command(seen, plans, "test", "Scala", "sbt", directory, "sbt test");
    }

    if directory.join("stack.yaml").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Haskell",
            "Stack",
            directory,
            "stack test",
        );
    } else if directory.join("cabal.project").is_file()
        || find_file_with_extension(directory, "cabal").is_some()
    {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Haskell",
            "Cabal",
            directory,
            "cabal test all",
        );
    }

    if directory.join("build.zig").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Zig",
            "zig build test",
            directory,
            "zig build test",
        );
    }

    if directory.join("dune-project").is_file() || directory.join("dune").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "OCaml",
            "Dune",
            directory,
            "dune runtest",
        );
    }

    if directory.join("rebar.config").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Erlang",
            "rebar3",
            directory,
            "rebar3 eunit",
        );
    }

    if directory.join("shard.yml").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Crystal",
            "spec",
            directory,
            "crystal spec",
        );
    }

    if find_file_with_extension(directory, "nimble").is_some() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Nim",
            "nimble",
            directory,
            "nimble test",
        );
    }

    if directory.join("Project.toml").is_file() && directory.join("test").is_dir() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Julia",
            "Pkg.test",
            directory,
            "julia --project -e \"using Pkg; Pkg.test()\"",
        );
    }

    if directory.join("DESCRIPTION").is_file() && directory.join("tests/testthat").is_dir() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "R",
            "testthat",
            directory,
            "Rscript -e \"testthat::test_dir('tests/testthat')\"",
        );
    }

    if directory.join("Makefile.PL").is_file()
        || directory.join("cpanfile").is_file()
        || (directory.join("t").is_dir()
            && dir_has_extension_recursive(
                &directory.join("t"),
                "t",
                AI_TEST_HEALTH_SCAN_MAX_DEPTH,
            ))
    {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Perl",
            "prove",
            directory,
            "prove -lr t",
        );
    }

    if let Some(ctest_dir) = ctest_working_dir(directory) {
        push_test_health_plan(
            seen,
            plans,
            TestHealthPlan {
                kind: "test",
                language: "C/C++",
                framework: "CTest",
                working_dir: ctest_dir,
                command: "ctest --output-on-failure".to_string(),
            },
        );
    }

    if has_make_target(directory, "test") {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Project",
            "Make",
            directory,
            "make test",
        );
    }

    if has_just_recipe(directory, "test") {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Project",
            "just",
            directory,
            "just test",
        );
    }

    if has_taskfile_task(directory, "test") {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Project",
            "Taskfile",
            directory,
            "task test",
        );
    }
}

fn collect_test_health_scan_dirs(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut seen = BTreeSet::new();

    while let Some((directory, depth)) = queue.pop_front() {
        let key = normalize_watch_path_for_compare(&directory);
        if !seen.insert(key) {
            continue;
        }
        result.push(directory.clone());
        if depth >= AI_TEST_HEALTH_SCAN_MAX_DEPTH {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        let mut child_dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || is_ignored_test_health_scan_dir(&path) {
                continue;
            }
            child_dirs.push(path);
        }
        child_dirs.sort();
        queue.extend(child_dirs.into_iter().map(|path| (path, depth + 1)));
    }

    result
}

fn push_test_health_plan(
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
    plan: TestHealthPlan,
) {
    let key = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        normalize_watch_path_for_compare(&plan.working_dir),
        plan.kind,
        plan.framework,
        plan.command
    );
    if seen.insert(key) {
        plans.push(plan);
    }
}

fn push_test_health_command(
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
    kind: &'static str,
    language: &'static str,
    framework: &'static str,
    working_dir: &Path,
    command: impl Into<String>,
) {
    push_test_health_plan(
        seen,
        plans,
        TestHealthPlan {
            kind,
            language,
            framework,
            working_dir: working_dir.to_path_buf(),
            command: command.into(),
        },
    );
}

fn add_package_validation_plans(
    directory: &Path,
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
) {
    let Some(scripts) = package_json_scripts(directory) else {
        return;
    };

    for script in [
        "test:ci",
        "test:unit",
        "test:integration",
        "test:e2e",
        "unit",
        "spec",
    ] {
        if has_valid_package_script(&scripts, script) {
            push_test_health_command(
                seen,
                plans,
                "test",
                "JavaScript/TypeScript",
                "package.json test script",
                directory,
                package_manager_script_command(directory, script),
            );
        }
    }

    for (script, kind, framework) in [
        ("typecheck", "typecheck", "package.json typecheck script"),
        ("check", "check", "package.json check script"),
        ("lint", "lint", "package.json lint script"),
        ("build", "build", "package.json build script"),
    ] {
        if has_valid_package_script(&scripts, script) {
            push_test_health_command(
                seen,
                plans,
                kind,
                "JavaScript/TypeScript",
                framework,
                directory,
                package_manager_script_command(directory, script),
            );
        }
    }
}

fn package_json_scripts(directory: &Path) -> Option<serde_json::Map<String, Value>> {
    read_json_file(&directory.join("package.json"))
        .and_then(|value| value.get("scripts").and_then(Value::as_object).cloned())
}

fn has_valid_package_script(scripts: &serde_json::Map<String, Value>, name: &str) -> bool {
    scripts
        .get(name)
        .and_then(Value::as_str)
        .is_some_and(is_meaningful_package_script)
}

fn is_meaningful_package_script(script: &str) -> bool {
    let script = script.trim().to_ascii_lowercase();
    !(script.is_empty()
        || script.contains("no test specified")
        || script.contains("echo \"error:")
        || script == "exit 1"
        || (script.starts_with("echo") && script.ends_with("exit 1"))
        || is_package_watch_script(&script))
}

fn is_package_watch_script(script: &str) -> bool {
    let is_explicit_false = script.contains("--watch=false")
        || script.contains("--watch false")
        || script.contains("--watchall=false")
        || script.contains("--watchall false")
        // Runners explicitly put into one-shot mode are NOT watch scripts.
        || script.contains("--run")
        || script.contains("--ci")
        || script.contains("run ");
    !is_explicit_false
        && (script.contains("--watch")
            || script == "watch"
            || script.starts_with("watch ")
            || script.ends_with(" watch")
            || script.contains(" watch ")
            || script.contains(" watch:")
            || script
                .split_whitespace()
                .any(|token| token.starts_with("watch:"))
            || is_watch_default_runner(script))
}

/// True when a script invokes a runner that defaults to WATCH mode unless told
/// otherwise. `vitest` with no subcommand watches by default; Create React App's
/// `react-scripts test` watches unless `CI=true` or `--watchAll=false`. Both would
/// otherwise hang the one-shot health check until it is force-killed.
fn is_watch_default_runner(script: &str) -> bool {
    let tokens: Vec<&str> = script.split_whitespace().collect();
    // `vitest` / `npx vitest` with no `run`/`watch` subcommand → watch default.
    let vitest_idx = tokens
        .iter()
        .position(|t| *t == "vitest" || t.ends_with("/vitest"));
    if let Some(index) = vitest_idx {
        let sub = tokens.get(index + 1).copied().unwrap_or("");
        let is_oneshot_sub = matches!(sub, "run" | "bench" | "list" | "related");
        if !is_oneshot_sub {
            return true;
        }
    }
    // Create React App test runner watches unless CI / --watchAll=false (handled
    // by the explicit-false check above).
    if script.contains("react-scripts test") {
        return true;
    }
    false
}

fn has_package_test_script(directory: &Path) -> bool {
    package_json_scripts(directory)
        .is_some_and(|scripts| has_valid_package_script(&scripts, "test"))
}

fn has_composer_test_script(directory: &Path) -> bool {
    let Some(value) = read_json_file(&directory.join("composer.json")) else {
        return false;
    };
    value
        .get("scripts")
        .and_then(Value::as_object)
        .and_then(|scripts| scripts.get("test"))
        .is_some()
}

fn read_json_file(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn is_python_test_project(directory: &Path) -> bool {
    directory.join("pytest.ini").is_file()
        || directory.join("tox.ini").is_file()
        || directory.join("noxfile.py").is_file()
        || directory.join("pyproject.toml").is_file()
        || directory.join("setup.cfg").is_file()
}

fn python_test_command(directory: &Path) -> String {
    if directory.join("tox.ini").is_file() {
        "python -m tox".to_string()
    } else if directory.join("noxfile.py").is_file() {
        "python -m nox".to_string()
    } else {
        "python -m pytest".to_string()
    }
}

fn ancestor_is_cargo_workspace(root: &Path, directory: &Path) -> bool {
    directory
        .ancestors()
        .skip(1)
        .take_while(|ancestor| ancestor.starts_with(root))
        .any(cargo_manifest_has_workspace)
}

fn cargo_manifest_has_workspace(directory: &Path) -> bool {
    std::fs::read_to_string(directory.join("Cargo.toml"))
        .is_ok_and(|content| content.lines().any(|line| line.trim() == "[workspace]"))
}

fn is_gradle_project(directory: &Path) -> bool {
    directory.join("build.gradle").is_file()
        || directory.join("build.gradle.kts").is_file()
        || directory.join("settings.gradle").is_file()
        || directory.join("settings.gradle.kts").is_file()
}

fn gradle_test_command(directory: &Path) -> String {
    if cfg!(windows) && directory.join("gradlew.bat").is_file() {
        "gradlew.bat test".to_string()
    } else if directory.join("gradlew").is_file() {
        "./gradlew test".to_string()
    } else {
        "gradle test".to_string()
    }
}

fn package_manager_script_command(directory: &Path, script: &str) -> String {
    if directory.join("pnpm-lock.yaml").is_file()
        || nearest_parent_has_file(directory, "pnpm-lock.yaml")
    {
        format!("pnpm {script}")
    } else if directory.join("yarn.lock").is_file()
        || nearest_parent_has_file(directory, "yarn.lock")
    {
        format!("yarn {script}")
    } else if directory.join("bun.lockb").is_file()
        || directory.join("bun.lock").is_file()
        || nearest_parent_has_file(directory, "bun.lockb")
        || nearest_parent_has_file(directory, "bun.lock")
    {
        format!("bun run {script}")
    } else if script == "test" {
        "npm test".to_string()
    } else {
        format!("npm run {script}")
    }
}

fn maven_manifest_has_modules(directory: &Path) -> bool {
    std::fs::read_to_string(directory.join("pom.xml"))
        .is_ok_and(|content| content.contains("<modules>") && content.contains("<module>"))
}

fn has_gradle_settings(directory: &Path) -> bool {
    directory.join("settings.gradle").is_file() || directory.join("settings.gradle.kts").is_file()
}

fn dotnet_test_command(directory: &Path) -> String {
    find_file_with_extension(directory, "sln").map_or_else(
        || {
            find_file_with_extension(directory, "csproj").map_or_else(
                || "dotnet test".to_string(),
                |project| format!("dotnet test {}", shell_quote_path(&project)),
            )
        },
        |solution| format!("dotnet test {}", shell_quote_path(&solution)),
    )
}

fn dart_test_command(directory: &Path) -> String {
    std::fs::read_to_string(directory.join("pubspec.yaml")).map_or_else(
        |_| "dart test".to_string(),
        |content| {
            if content.lines().any(|line| line.trim() == "flutter:") {
                "flutter test".to_string()
            } else {
                "dart test".to_string()
            }
        },
    )
}

fn ruby_test_command(directory: &Path) -> &'static str {
    if directory.join("Rakefile").is_file() {
        "bundle exec rake test"
    } else {
        "bundle exec rspec"
    }
}

fn ctest_working_dir(directory: &Path) -> Option<PathBuf> {
    if directory.join("CTestTestfile.cmake").is_file() {
        return Some(directory.to_path_buf());
    }

    for child in [
        "build",
        "cmake-build-debug",
        "cmake-build-release",
        "out/build",
    ] {
        let candidate = directory.join(child);
        if candidate.join("CTestTestfile.cmake").is_file() {
            return Some(candidate);
        }
    }

    None
}

fn has_make_target(directory: &Path, target: &str) -> bool {
    ["Makefile", "makefile", "GNUmakefile"]
        .iter()
        .any(|file_name| file_has_recipe_target(&directory.join(file_name), target))
}

fn has_just_recipe(directory: &Path, target: &str) -> bool {
    ["justfile", "Justfile", ".justfile"]
        .iter()
        .any(|file_name| file_has_recipe_target(&directory.join(file_name), target))
}

fn has_taskfile_task(directory: &Path, target: &str) -> bool {
    [
        "Taskfile.yml",
        "Taskfile.yaml",
        "Taskfile.dist.yml",
        "Taskfile.dist.yaml",
    ]
    .iter()
    .any(|file_name| taskfile_has_task(&directory.join(file_name), target))
}

fn file_has_recipe_target(path: &Path, target: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('#')
            && trimmed.starts_with(target)
            && trimmed[target.len()..].trim_start().starts_with(':')
    })
}

fn taskfile_has_task(path: &Path, target: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let target_line = format!("  {target}:");
    content
        .lines()
        .any(|line| line == target_line || line.trim_start() == format!("{target}:"))
}

fn find_file_with_extension(directory: &Path, extension: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(directory).ok()?;
    let mut matches = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case(extension))
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches.into_iter().next()
}

fn dir_has_extension_recursive(directory: &Path, extension: &str, max_depth: usize) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    let mut subdirs = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_file() {
            if path
                .extension()
                .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case(extension))
            {
                return true;
            }
        } else if max_depth > 0 && path.is_dir() {
            subdirs.push(path);
        }
    }
    subdirs
        .into_iter()
        .any(|subdir| dir_has_extension_recursive(&subdir, extension, max_depth - 1))
}

fn nearest_parent_has_file(directory: &Path, file_name: &str) -> bool {
    directory
        .ancestors()
        .skip(1)
        .take(4)
        .any(|parent| parent.join(file_name).is_file())
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | ':'))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

fn is_ignored_test_health_scan_dir(path: &Path) -> bool {
    let Some(name) = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
    else {
        return true;
    };
    WATCH_EXCLUDED_COMPONENTS.contains(&name.as_str())
        || matches!(
            name.as_str(),
            ".cache"
                | ".gradle"
                | ".idea"
                | ".pytest_cache"
                | ".ruff_cache"
                | ".venv"
                | ".vscode"
                | "__pycache__"
                | "build"
                | "out"
                | "venv"
        )
}

fn same_path(left: &Path, right: &Path) -> bool {
    normalize_watch_path_for_compare(left) == normalize_watch_path_for_compare(right)
}

fn relative_depth(root: &Path, path: &Path) -> usize {
    path.strip_prefix(root).ok().map_or(usize::MAX, |relative| {
        relative
            .components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .count()
    })
}

async fn run_test_health_plans(
    root: PathBuf,
    plans: Vec<TestHealthPlan>,
) -> Result<TestHealthResponse, String> {
    if plans.is_empty() {
        return Ok(empty_test_health_response(root));
    }

    let started = std::time::Instant::now();
    let skipped = plans.len().saturating_sub(AI_TEST_HEALTH_MAX_RUNNERS);
    let mut runners = Vec::new();
    for plan in plans.into_iter().take(AI_TEST_HEALTH_MAX_RUNNERS) {
        runners.push(run_single_test_health_plan(&root, plan).await);
    }
    let total_duration_ms = started.elapsed().as_millis();
    Ok(test_health_response_from_runners(
        root,
        runners,
        skipped,
        total_duration_ms,
    ))
}

async fn run_single_test_health_plan(root: &Path, plan: TestHealthPlan) -> TestHealthRunnerResult {
    let started = std::time::Instant::now();
    let mut command = shell_command(&plan.command);
    command.current_dir(&plan.working_dir);
    // Capture pipes so a watch-mode runner (which never closes stdout) can't wedge
    // the read, and make the child die with its handle so a timeout actually frees
    // it instead of leaking an orphaned watcher.
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true);

    let output_result = match command.spawn() {
        Ok(mut child) => {
            // Drain both pipes concurrently so a chatty runner can't fill a pipe and
            // deadlock; `wait_with_output` would move `child`, leaving nothing to kill
            // on timeout, so read the handles separately and keep `child` to kill.
            let child_pid = child.id();
            let mut stdout_pipe = child.stdout.take();
            let mut stderr_pipe = child.stderr.take();
            let collect = async {
                use tokio::io::AsyncReadExt;
                let mut out = Vec::new();
                let mut err = Vec::new();
                if let Some(pipe) = stdout_pipe.as_mut() {
                    let _ = pipe.read_to_end(&mut out).await;
                }
                if let Some(pipe) = stderr_pipe.as_mut() {
                    let _ = pipe.read_to_end(&mut err).await;
                }
                let status = child.wait().await;
                (status, out, err)
            };
            match timeout(Duration::from_secs(AI_TEST_HEALTH_TIMEOUT_SECS), collect).await {
                Ok((Ok(status), out, err)) => Ok(Ok(CollectedOutput {
                    status,
                    stdout: out,
                    stderr: err,
                })),
                Ok((Err(error), _, _)) => Ok(Err(error)),
                Err(elapsed) => {
                    // Timed out: kill the whole process tree (the shell plus any test
                    // runner / watcher it spawned) so nothing survives this call.
                    kill_test_health_process_tree(child_pid).await;
                    Err(elapsed)
                }
            }
        }
        Err(error) => Ok(Err(error)),
    };
    let duration_ms = started.elapsed().as_millis();
    let id = test_health_runner_id(root, &plan);
    let workspace_relative_path = workspace_relative_path(root, &plan.working_dir);

    match output_result {
        Ok(Ok(output)) => TestHealthRunnerResult {
            id,
            workspace_relative_path,
            status: if output.status.success() {
                "passed".to_string()
            } else {
                "failed".to_string()
            },
            kind: plan.kind.to_string(),
            language: plan.language.to_string(),
            framework: plan.framework.to_string(),
            command: plan.command,
            exit_code: output.status.code(),
            duration_ms,
            stdout: truncate_output(&String::from_utf8_lossy(&output.stdout)),
            stderr: truncate_output(&String::from_utf8_lossy(&output.stderr)),
            timed_out: false,
        },
        Ok(Err(error)) => TestHealthRunnerResult {
            id,
            workspace_relative_path,
            status: "error".to_string(),
            kind: plan.kind.to_string(),
            language: plan.language.to_string(),
            framework: plan.framework.to_string(),
            command: plan.command,
            exit_code: None,
            duration_ms,
            stdout: String::new(),
            stderr: format!("Failed to start test command: {error}"),
            timed_out: false,
        },
        Err(_) => TestHealthRunnerResult {
            id,
            workspace_relative_path,
            status: "timeout".to_string(),
            kind: plan.kind.to_string(),
            language: plan.language.to_string(),
            framework: plan.framework.to_string(),
            command: plan.command,
            exit_code: None,
            duration_ms,
            stdout: String::new(),
            stderr: format!("Test command timed out after {AI_TEST_HEALTH_TIMEOUT_SECS} seconds"),
            timed_out: true,
        },
    }
}

fn empty_test_health_response(root: PathBuf) -> TestHealthResponse {
    TestHealthResponse {
        workspace_root: root,
        status: "skipped".to_string(),
        summary: TestHealthSummary {
            total: 0,
            passed: 0,
            failed: 0,
            timed_out: 0,
            errored: 0,
            skipped: 0,
            duration_ms: 0,
        },
        runners: Vec::new(),
        language: "Mixed".to_string(),
        framework: "No supported test runner".to_string(),
        command: String::new(),
        exit_code: None,
        duration_ms: 0,
        stdout: String::new(),
        stderr: "No supported test runner was detected in the workspace.".to_string(),
        timed_out: false,
    }
}

fn test_health_response_from_runners(
    root: PathBuf,
    runners: Vec<TestHealthRunnerResult>,
    skipped: usize,
    duration_ms: u128,
) -> TestHealthResponse {
    let summary = TestHealthSummary {
        total: runners.len() + skipped,
        passed: runners
            .iter()
            .filter(|runner| runner.status == "passed")
            .count(),
        failed: runners
            .iter()
            .filter(|runner| runner.status == "failed")
            .count(),
        timed_out: runners
            .iter()
            .filter(|runner| runner.status == "timeout")
            .count(),
        errored: runners
            .iter()
            .filter(|runner| runner.status == "error")
            .count(),
        skipped,
        duration_ms,
    };
    let status = aggregate_test_health_status(&summary);
    let primary = runners
        .iter()
        .find(|runner| runner.status != "passed")
        .or_else(|| runners.first());
    let language = aggregate_test_health_language(&runners);
    let framework = if runners.len() == 1 {
        primary
            .map(|runner| runner.framework.clone())
            .unwrap_or_default()
    } else {
        format!("{} runners", runners.len())
    };
    let command = if runners.len() == 1 {
        primary
            .map(|runner| runner.command.clone())
            .unwrap_or_default()
    } else {
        runners
            .iter()
            .take(4)
            .map(|runner| format!("{}: {}", runner.workspace_relative_path, runner.command))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let exit_code = primary.and_then(|runner| runner.exit_code);
    let timed_out = summary.timed_out > 0;
    let stdout = aggregate_test_stream(&runners, false);
    let stderr = aggregate_test_stream(&runners, true);

    TestHealthResponse {
        workspace_root: root,
        status,
        summary,
        runners,
        language,
        framework,
        command,
        exit_code,
        duration_ms,
        stdout,
        stderr,
        timed_out,
    }
}

fn aggregate_test_health_status(summary: &TestHealthSummary) -> String {
    if summary.failed > 0 {
        "failed".to_string()
    } else if summary.timed_out > 0 {
        "timeout".to_string()
    } else if summary.errored > 0 {
        "error".to_string()
    } else if summary.passed > 0 && summary.skipped == 0 {
        "passed".to_string()
    } else if summary.passed > 0 {
        "partial".to_string()
    } else {
        "skipped".to_string()
    }
}

fn aggregate_test_health_language(runners: &[TestHealthRunnerResult]) -> String {
    let languages = runners
        .iter()
        .map(|runner| runner.language.as_str())
        .collect::<BTreeSet<_>>();
    if languages.len() == 1 {
        languages.into_iter().next().unwrap_or("Mixed").to_string()
    } else {
        "Mixed".to_string()
    }
}

fn aggregate_test_stream(runners: &[TestHealthRunnerResult], stderr: bool) -> String {
    let mut sections = Vec::new();
    for runner in runners {
        let output = if stderr {
            &runner.stderr
        } else {
            &runner.stdout
        };
        if output.trim().is_empty() {
            continue;
        }
        sections.push(format!(
            "## {} [{} / {}]\n{}",
            runner.workspace_relative_path, runner.kind, runner.framework, output
        ));
    }
    truncate_output(&sections.join("\n\n"))
}

fn test_health_runner_id(root: &Path, plan: &TestHealthPlan) -> String {
    format!(
        "{}:{}:{}",
        workspace_relative_path(root, &plan.working_dir),
        plan.kind,
        plan.framework
            .to_ascii_lowercase()
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "-")
    )
}

fn workspace_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map_or_else(
            || ".".to_string(),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        )
}

fn truncate_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= AI_TEST_HEALTH_MAX_OUTPUT_CHARS {
        return trimmed.to_string();
    }
    let head: String = trimmed
        .chars()
        .take(AI_TEST_HEALTH_MAX_OUTPUT_CHARS)
        .collect();
    format!("{head}\n...[truncated]")
}

/// Pipe output collected from a finished child, mirroring the shape of
/// `std::process::Output` we consume (status + raw stdout/stderr).
struct CollectedOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn shell_command(command_line: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/C").arg(command_line);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg(command_line);
        // Own process group (pgid == pid) so a timeout can group-kill the shell and
        // every test runner / watcher it spawned in one shot.
        command.process_group(0);
        command
    }
}

/// Kill the timed-out test process and everything it spawned. A `sh -c "vitest"`
/// (or jest/CRA) often launches a watcher child; killing only the shell would
/// orphan it, so target the whole tree/group.
async fn kill_test_health_process_tree(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    #[cfg(windows)]
    {
        let mut command = tokio::process::Command::new("taskkill");
        command
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW);
        let _ = command.output().await;
    }
    #[cfg(not(windows))]
    {
        // The child leads its own process group (process_group(0)), so its pgid
        // equals its pid; the leading '-' signals the whole group.
        let _ = tokio::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .output()
            .await;
    }
}

fn normalize_watch_path_for_compare(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn watch_default_runners_are_treated_as_watch_scripts() {
        // Runners that default to watch with no explicit run flag would hang the
        // one-shot health check, so they must be filtered out.
        assert!(is_package_watch_script("vitest"));
        assert!(is_package_watch_script("npx vitest"));
        assert!(is_package_watch_script("react-scripts test"));
        // Explicit one-shot invocations are fine.
        assert!(!is_package_watch_script("vitest run"));
        assert!(!is_package_watch_script("vitest run --coverage"));
        assert!(!is_package_watch_script("jest --ci"));
        assert!(!is_package_watch_script(
            "react-scripts test --watchall=false"
        ));
        // A plain non-watch command is unaffected.
        assert!(!is_package_watch_script("jest"));
        assert!(!is_package_watch_script("cargo test"));
    }

    #[test]
    fn watch_default_runners_are_not_meaningful_scripts() {
        assert!(!is_meaningful_package_script("vitest"));
        assert!(is_meaningful_package_script("vitest run"));
        assert!(is_meaningful_package_script("jest"));
    }

    #[test]
    fn test_health_detects_root_workspace_before_nested_crates() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("crates/example")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/example\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/example/Cargo.toml"),
            "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let plans = detect_test_health_plans(&root);

        assert_eq!(plans.len(), 1);
        assert!(same_path(&plans[0].working_dir, &root));
        assert_eq!(plans[0].command, "cargo test --workspace");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_detects_nested_projects_when_root_has_no_runner() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        let app = root.join("apps/web");
        let api = root.join("services/api");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&api).unwrap();
        std::fs::write(
            app.join("package.json"),
            r#"{"scripts":{"test":"vitest run"}}"#,
        )
        .unwrap();
        std::fs::write(api.join("go.mod"), "module example.com/api\n").unwrap();

        let plans = detect_test_health_plans(&root);
        let commands = plans
            .iter()
            .map(|plan| plan.command.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(plans.len(), 2);
        assert!(commands.contains("npm test"));
        assert!(commands.contains("go test ./..."));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_detects_package_validation_scripts_without_test_script() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        let app = root.join("apps/desktop");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        std::fs::write(
            app.join("package.json"),
            r#"{"scripts":{"typecheck":"tsc --noEmit","build":"vite build"}}"#,
        )
        .unwrap();

        let plans = detect_test_health_plans(&root);
        let commands = plans
            .iter()
            .map(|plan| (plan.kind, plan.command.as_str()))
            .collect::<BTreeSet<_>>();

        assert_eq!(plans.len(), 2);
        assert!(commands.contains(&("typecheck", "pnpm typecheck")));
        assert!(commands.contains(&("build", "pnpm build")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_keeps_package_validation_next_to_rust_workspace() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("crates/example")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/example\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/example/Cargo.toml"),
            "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"scripts":{"typecheck":"tsc --noEmit"}}"#,
        )
        .unwrap();

        let plans = detect_test_health_plans(&root);
        let commands = plans
            .iter()
            .map(|plan| (plan.kind, plan.command.as_str()))
            .collect::<BTreeSet<_>>();

        assert_eq!(plans.len(), 2);
        assert!(commands.contains(&("test", "cargo test --workspace")));
        assert!(commands.contains(&("typecheck", "pnpm typecheck")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_detects_generic_project_test_targets() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("Makefile"), "test:\n\t./run-tests\n").unwrap();
        std::fs::write(root.join("justfile"), "test:\n    ./run-tests\n").unwrap();
        std::fs::write(
            root.join("Taskfile.yml"),
            "version: '3'\ntasks:\n  test:\n    cmds: ['echo ok']\n",
        )
        .unwrap();
        std::fs::write(
            root.join("build/CTestTestfile.cmake"),
            "# CTest generated file\n",
        )
        .unwrap();

        let plans = detect_test_health_plans(&root);
        let commands = plans
            .iter()
            .map(|plan| plan.command.as_str())
            .collect::<BTreeSet<_>>();

        assert!(commands.contains("make test"));
        assert!(commands.contains("just test"));
        assert!(commands.contains("task test"));
        assert!(commands.contains("ctest --output-on-failure"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_response_aggregates_failure_before_success() {
        let root = PathBuf::from("C:/work/project");
        let runners = vec![
            TestHealthRunnerResult {
                id: ".:cargo".to_string(),
                workspace_relative_path: ".".to_string(),
                status: "passed".to_string(),
                kind: "test".to_string(),
                language: "Rust".to_string(),
                framework: "Cargo".to_string(),
                command: "cargo test".to_string(),
                exit_code: Some(0),
                duration_ms: 10,
                stdout: "ok".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
            TestHealthRunnerResult {
                id: "apps/web:package".to_string(),
                workspace_relative_path: "apps/web".to_string(),
                status: "failed".to_string(),
                kind: "test".to_string(),
                language: "JavaScript/TypeScript".to_string(),
                framework: "package.json test script".to_string(),
                command: "pnpm test".to_string(),
                exit_code: Some(1),
                duration_ms: 20,
                stdout: String::new(),
                stderr: "failed".to_string(),
                timed_out: false,
            },
        ];

        let response = test_health_response_from_runners(root, runners, 0, 30);

        assert_eq!(response.status, "failed");
        assert_eq!(response.summary.total, 2);
        assert_eq!(response.summary.passed, 1);
        assert_eq!(response.summary.failed, 1);
        assert_eq!(response.exit_code, Some(1));
        assert_eq!(response.language, "Mixed");
    }
}
