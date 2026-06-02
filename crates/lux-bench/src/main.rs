#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use lux_core::SearchOptions;
use serde::Serialize;
use uuid::Uuid;

const FIXTURE_DIRECTORIES: usize = 24;
const FILES_PER_DIRECTORY: usize = 60;
const SOURCE_FILES: usize = FIXTURE_DIRECTORIES * FILES_PER_DIRECTORY;
const ROOT_METADATA_FILES: usize = 2;
const EXPECTED_LISTED_FILES: usize = SOURCE_FILES + ROOT_METADATA_FILES + 1;
const FILE_LINES: usize = 28;
const EXPECTED_SEARCH_HITS: usize = SOURCE_FILES / 4;

const LIST_FILES_THRESHOLD_MS: u128 = 1_500;
const SEARCH_THRESHOLD_MS: u128 = 2_000;
const EVENT_BATCH_THRESHOLD_MS: u128 = 80;

type BenchResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkReport {
    fixture: BenchmarkFixtureReport,
    metrics: Vec<BenchmarkMetric>,
    thresholds: Vec<BenchmarkThreshold>,
    passed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkFixtureReport {
    root: PathBuf,
    files: usize,
    directories: usize,
    lines_per_file: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkMetric {
    id: &'static str,
    duration_ms: u128,
    items: usize,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkThreshold {
    metric_id: &'static str,
    max_ms: u128,
    actual_ms: u128,
    passed: bool,
}

#[derive(Debug, Clone)]
struct BenchmarkOptions {
    assert_thresholds: bool,
    keep_fixture: bool,
    output_path: Option<PathBuf>,
}

fn main() -> BenchResult<()> {
    let options = parse_options()?;
    let fixture = BenchmarkFixture::create()?;
    let report = run_benchmarks(&fixture)?;
    let report_json = serde_json::to_string_pretty(&report)?;

    println!("{report_json}");
    if let Some(output_path) = options.output_path.as_deref() {
        write_report(output_path, &report_json)?;
    }

    if !options.keep_fixture {
        fixture.cleanup()?;
    }

    if options.assert_thresholds && !report.passed {
        return Err("Lux performance benchmark thresholds failed".into());
    }

    Ok(())
}

fn parse_options() -> BenchResult<BenchmarkOptions> {
    let mut options = BenchmarkOptions {
        assert_thresholds: false,
        keep_fixture: false,
        output_path: None,
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--assert" => options.assert_thresholds = true,
            "--keep-fixture" => options.keep_fixture = true,
            "--output" => {
                let Some(path) = args.next() else {
                    return Err("--output requires a path".into());
                };
                options.output_path = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                if let Some(path) = other.strip_prefix("--output=") {
                    if path.is_empty() {
                        return Err("--output requires a path".into());
                    }
                    options.output_path = Some(PathBuf::from(path));
                } else {
                    return Err(format!("unknown lux-bench argument: {other}").into());
                }
            }
        }
    }

    Ok(options)
}

fn print_help() {
    println!(
        "lux-bench\n\nUsage: cargo run -p lux-bench -- [--assert] [--keep-fixture] [--output <path>]\n\n--assert         Fail when benchmark thresholds are exceeded.\n--keep-fixture   Keep generated fixture under the temp directory.\n--output <path>  Write the benchmark JSON report to a file."
    );
}

fn write_report(path: &Path, report_json: &str) -> BenchResult<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{report_json}\n"))?;
    Ok(())
}

fn run_benchmarks(fixture: &BenchmarkFixture) -> BenchResult<BenchmarkReport> {
    let metrics = vec![
        benchmark_list_files(&fixture.root)?,
        benchmark_search(&fixture.root)?,
        benchmark_event_batching()?,
    ];
    let thresholds = thresholds_for_metrics(&metrics);
    let passed = thresholds.iter().all(|threshold| threshold.passed);

    Ok(BenchmarkReport {
        fixture: BenchmarkFixtureReport {
            root: fixture.root.clone(),
            files: EXPECTED_LISTED_FILES,
            directories: FIXTURE_DIRECTORIES,
            lines_per_file: FILE_LINES,
        },
        metrics,
        thresholds,
        passed,
    })
}

fn benchmark_list_files(root: &Path) -> BenchResult<BenchmarkMetric> {
    let started = Instant::now();
    let files = lux_fs::list_files(root, EXPECTED_LISTED_FILES + 100)?;
    let duration = started.elapsed();

    if files.len() != EXPECTED_LISTED_FILES {
        return Err(format!(
            "list_files returned {} files, expected {EXPECTED_LISTED_FILES}",
            files.len()
        )
        .into());
    }

    Ok(BenchmarkMetric {
        id: "workspace.listFiles",
        duration_ms: duration.as_millis(),
        items: files.len(),
        detail: "ignore-aware file listing over generated workspace".to_string(),
    })
}

fn benchmark_search(root: &Path) -> BenchResult<BenchmarkMetric> {
    let options = SearchOptions {
        case_sensitive: true,
        include_globs: vec!["**/*.rs".to_string()],
        exclude_globs: vec!["target/**".to_string()],
        max_results: EXPECTED_SEARCH_HITS + 20,
        ..SearchOptions::default()
    };
    let response = lux_search::query(root, "lux_target_symbol".to_string(), &options)?;

    if response.hits.len() != EXPECTED_SEARCH_HITS {
        return Err(format!(
            "search returned {} hits, expected {EXPECTED_SEARCH_HITS}",
            response.hits.len()
        )
        .into());
    }

    Ok(BenchmarkMetric {
        id: "workspace.searchLiteral",
        duration_ms: response.elapsed_ms,
        items: response.hits.len(),
        detail: "literal code search with include glob over generated workspace".to_string(),
    })
}

fn benchmark_event_batching() -> BenchResult<BenchmarkMetric> {
    let events = synthetic_event_paths(12_000);
    let window = Duration::from_millis(180);
    let started = Instant::now();
    let batches = coalesce_workspace_events(&events, window);
    let duration = started.elapsed();
    let expected_batches = expected_event_batch_count(&events, window);

    if batches.len() != expected_batches {
        return Err(format!(
            "event batching returned {} batches, expected {expected_batches}",
            batches.len()
        )
        .into());
    }

    Ok(BenchmarkMetric {
        id: "workspace.eventBatching",
        duration_ms: duration.as_millis(),
        items: events.len(),
        detail: format!(
            "coalesced {} file events into {} refresh batches",
            events.len(),
            batches.len()
        ),
    })
}

fn expected_event_batch_count(events: &[(PathBuf, Duration)], window: Duration) -> usize {
    let Some((_, first_timestamp)) = events.first() else {
        return 0;
    };
    let Some((_, last_timestamp)) = events.last() else {
        return 0;
    };
    let span_ms = last_timestamp.saturating_sub(*first_timestamp).as_millis();
    usize::try_from((span_ms / window.as_millis()) + 1).unwrap_or(usize::MAX)
}

fn thresholds_for_metrics(metrics: &[BenchmarkMetric]) -> Vec<BenchmarkThreshold> {
    metrics
        .iter()
        .map(|metric| {
            let max_ms = match metric.id {
                "workspace.listFiles" => LIST_FILES_THRESHOLD_MS,
                "workspace.searchLiteral" => SEARCH_THRESHOLD_MS,
                "workspace.eventBatching" => EVENT_BATCH_THRESHOLD_MS,
                _ => u128::MAX,
            };
            BenchmarkThreshold {
                metric_id: metric.id,
                max_ms,
                actual_ms: metric.duration_ms,
                passed: metric.duration_ms <= max_ms,
            }
        })
        .collect()
}

fn synthetic_event_paths(count: usize) -> Vec<(PathBuf, Duration)> {
    (0..count)
        .map(|index| {
            let elapsed_ms = (index as u64 * 17) / 19;
            (
                PathBuf::from(format!("src/module-{}/file-{}.rs", index % 64, index % 512)),
                Duration::from_millis(elapsed_ms),
            )
        })
        .collect()
}

fn coalesce_workspace_events(
    events: &[(PathBuf, Duration)],
    window: Duration,
) -> Vec<Vec<PathBuf>> {
    let mut batches = Vec::<Vec<PathBuf>>::new();
    let mut current_batch = Vec::<PathBuf>::new();
    let mut current_window_end = None::<Duration>;

    for (path, timestamp) in events {
        let should_flush = current_window_end.is_some_and(|window_end| *timestamp > window_end);
        if should_flush && !current_batch.is_empty() {
            current_batch.sort();
            current_batch.dedup();
            batches.push(std::mem::take(&mut current_batch));
            current_window_end = None;
        }

        if current_window_end.is_none() {
            current_window_end = Some(*timestamp + window);
        }
        current_batch.push(path.clone());
    }

    if !current_batch.is_empty() {
        current_batch.sort();
        current_batch.dedup();
        batches.push(current_batch);
    }

    batches
}

struct BenchmarkFixture {
    root: PathBuf,
}

impl BenchmarkFixture {
    fn create() -> BenchResult<Self> {
        let root = env::temp_dir().join(format!("lux-bench-{}", Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        write_fixture(&root)?;
        Ok(Self { root })
    }

    fn cleanup(&self) -> BenchResult<()> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        Ok(())
    }
}

fn write_fixture(root: &Path) -> BenchResult<()> {
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"lux-bench-fixture\"\n",
    )?;
    fs::write(root.join(".gitignore"), "target\nnode_modules\n")?;

    for directory_index in 0..FIXTURE_DIRECTORIES {
        let directory = root
            .join("src")
            .join(format!("module-{directory_index:02}"));
        fs::create_dir_all(&directory)?;
        for file_index in 0..FILES_PER_DIRECTORY {
            let global_index = directory_index * FILES_PER_DIRECTORY + file_index;
            let file = directory.join(format!("file-{file_index:03}.rs"));
            fs::write(file, source_file_contents(global_index))?;
        }
    }

    let ignored_dir = root.join("target").join("generated");
    fs::create_dir_all(&ignored_dir)?;
    fs::write(
        ignored_dir.join("ignored.rs"),
        "pub const IGNORED: &str = \"lux_target_symbol\";\n",
    )?;
    Ok(())
}

fn source_file_contents(index: usize) -> String {
    let mut text = String::with_capacity(1_600);
    let _ = writeln!(text, "pub mod generated_{index} {{");
    for line in 0..FILE_LINES {
        let marker = if index.is_multiple_of(4) && line == 7 {
            "lux_target_symbol"
        } else {
            "ordinary_symbol"
        };
        let _ = writeln!(
            text,
            "    pub fn function_{line}_{index}() -> &'static str {{ \"{marker}_{line}_{index}\" }}"
        );
    }
    text.push_str("}\n");
    text
}
