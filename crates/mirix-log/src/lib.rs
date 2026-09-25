//! Process-wide file logging shared by daemon and standalone panel.
use anyhow::{Context, Result};
use flexi_logger::{
    Cleanup, Criterion, Duplicate, FileSpec, Logger, LoggerHandle, Naming, WriteMode,
};
use std::{io::Write, path::Path, time::Duration};

const FILTER: &str = "info,zbus=warn";

/// Keep the handle alive until all application workers have stopped.
pub fn init(program: &str) -> Result<LoggerHandle> {
    let directory = dirs::state_dir()
        .context("locate user state directory")?
        .join("mirix/logs");
    let handle = builder(&directory, program, 10 * 1024 * 1024)?
        .start()
        .context("start file logger")?;
    log::info!(
        "logging started program={program} version={} directory={} filter={}",
        env!("CARGO_PKG_VERSION"),
        directory.display(),
        std::env::var("RUST_LOG").unwrap_or_else(|_| FILTER.to_owned())
    );
    Ok(handle)
}

fn builder(directory: &Path, program: &str, rotate_bytes: u64) -> Result<Logger> {
    // State logs are private to the user, including when the directory is first created.
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    Ok(Logger::try_with_env_or_str(FILTER)?
        .log_to_file(FileSpec::default().directory(directory).basename(program))
        .format(format)
        .duplicate_to_stderr(Duplicate::Warn)
        .write_mode(WriteMode::AsyncWith {
            pool_capa: 50,
            message_capa: 1024,
            flush_interval: Duration::from_secs(1),
        })
        .rotate(
            Criterion::Size(rotate_bytes),
            Naming::TimestampsDirect,
            Cleanup::KeepLogFiles(7),
        ))
}

fn format(
    out: &mut dyn Write,
    now: &mut flexi_logger::DeferredNow,
    record: &log::Record<'_>,
) -> std::io::Result<()> {
    let thread = std::thread::current();
    write!(
        out,
        "{} {} pid={} thread={:?}/{} {} {}",
        now.format("%Y-%m-%dT%H:%M:%S%.6f%:z"),
        record.level(),
        std::process::id(),
        thread.id(),
        thread.name().unwrap_or("unnamed"),
        record.target(),
        record.args()
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn async_file_output_rotates_and_flushes_the_final_record() {
        let root = tempfile::tempdir().unwrap();
        let (logger, handle) = super::builder(root.path(), "test", 512)
            .unwrap()
            .cleanup_in_background_thread(false)
            .build()
            .unwrap();
        for sequence in 0..100 {
            logger.log(
                &log::Record::builder()
                    .level(log::Level::Info)
                    .target("mirix_log_test")
                    .args(format_args!("record sequence={sequence:03}"))
                    .build(),
            );
        }
        handle.shutdown();
        let files = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert!(
            files.len() > 1 && files.len() <= 7,
            "rotation files: {files:?}"
        );
        let text = files
            .iter()
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect::<String>();
        assert!(text.contains("sequence=099"));
        assert!(text.contains("pid="));
        assert!(text.contains("thread="));
        assert!(text.contains("mirix_log_test"));
        assert!(!text.contains("\u{1b}["));
    }
}
