//! Inspect configured candidates without opening personal learning storage.
use clap::Parser;
use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::{Engine, KeyEvent, KeyState, Modifiers};
use std::time::{Duration, Instant};

#[derive(Parser)]
struct Cli {
    #[arg(required = true)]
    inputs: Vec<String>,
    #[arg(long, default_value_t = 5)]
    limit: usize,
    #[arg(long)]
    config_dir: Option<std::path::PathBuf>,
    /// Repeat each input in memory, without recording selections.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    repeat: u32,
    /// Report per-prefix key latency and candidate paging separately.
    #[arg(long)]
    timings: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let started = std::time::Instant::now();
    let config = cli
        .config_dir
        .map(Ok)
        .unwrap_or_else(PinyinEngine::default_config_dir)?;
    let mut engine = PinyinEngine::from_config(config)?;
    eprintln!("dictionary startup: {:?}", started.elapsed());
    for input in cli.inputs {
        let mut keys = input
            .char_indices()
            .map(|(offset, character)| {
                (
                    input[..offset + character.len_utf8()].to_owned(),
                    KeyEvent {
                        keycode: 0,
                        keysym: character as u32,
                        state: KeyState::Press,
                        mods: Modifiers::default(),
                        text: Some(character.to_string()),
                    },
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        let mut totals = Vec::new();
        let mut pages = Vec::new();
        for _ in 0..cli.repeat {
            engine.reset();
            let started = Instant::now();
            for (_, event, samples) in &mut keys {
                let key_started = Instant::now();
                engine.process_key(event);
                samples.push(key_started.elapsed());
            }
            totals.push(started.elapsed());
            if cli.timings {
                let page = KeyEvent {
                    keycode: 0,
                    keysym: mirix_engine_protocol::keysym::PAGE_DOWN,
                    state: KeyState::Press,
                    mods: Modifiers::default(),
                    text: None,
                };
                let started = Instant::now();
                engine.process_key(&page);
                pages.push(started.elapsed());
            }
        }
        println!("{input}:");
        for candidate in engine.candidates().iter().take(cli.limit) {
            println!(
                "  {}\t{}",
                candidate.text,
                candidate.comment.as_deref().unwrap_or_default()
            );
        }
        report(&input, &mut totals);
        if cli.timings {
            for (prefix, _, samples) in &mut keys {
                report(&format!("  key {prefix}"), samples);
            }
            report("  page down (engine only)", &mut pages);
        }
    }
    Ok(())
}

fn report(label: &str, samples: &mut [Duration]) {
    let first = samples[0];
    samples.sort_unstable();
    eprintln!(
        "{label}: first={first:?} p50={:?} p95={:?} max={:?} n={}",
        samples[samples.len() / 2],
        samples[(samples.len() * 95).div_ceil(100) - 1],
        samples[samples.len() - 1],
        samples.len()
    );
}
