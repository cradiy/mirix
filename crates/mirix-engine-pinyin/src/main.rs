use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;

use clap::Parser;

use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::serve;

#[derive(Parser)]
#[command(about = "Mirix pinyin input method")]
struct Cli {
    /// Profile and learning data root. Defaults to the Mirix user data directory.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Shared and profile dictionary root. Overrides the host-provided directory.
    #[arg(long)]
    config_dir: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let cli = Cli::parse();
    let config = cli
        .config_dir
        .or_else(|| std::env::var_os("MIRIX_CONFIG_DIR").map(PathBuf::from))
        .map(Ok)
        .unwrap_or_else(PinyinEngine::default_config_dir)?;
    let data = cli
        .data_dir
        .or_else(|| std::env::var_os("MIRIX_DATA_DIR").map(PathBuf::from))
        .or_else(|| dirs::data_dir().map(|path| path.join("mirix/input-method-data/pinyin")))
        .ok_or_else(|| io::Error::other("user data directory unavailable"))?;
    let mut engine = PinyinEngine::open(data, config)?;
    serve(&mut engine, &mut reader, &mut writer)?;
    Ok(())
}
