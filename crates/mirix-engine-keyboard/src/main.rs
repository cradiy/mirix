use std::io::{self, BufReader, BufWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = mirix_engine_keyboard::KeyboardEngine::new();
    mirix_engine_protocol::serve(
        &mut engine,
        &mut BufReader::new(io::stdin().lock()),
        &mut BufWriter::new(io::stdout().lock()),
    )?;
    Ok(())
}
