//! Enumerate ambiguous whole syllables from the actual parser and character table.
//! Pass the character dictionary path as the positional argument.
use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::Parser as _;

use mirix_pinyin_core::{
    EdgeKind, ParseGraph, Parser, SyllableId, SyllableTable, import_character_tsv,
};

#[derive(clap::Parser)]
#[command(about = "Enumerate ambiguous syllables using a character dictionary")]
struct Cli {
    /// Character dictionary file to inspect.
    characters: PathBuf,
}

fn spelling(table: &SyllableTable, path: &[SyllableId]) -> String {
    path.iter()
        .map(|id| table.spelling(*id).unwrap())
        .collect::<Vec<_>>()
        .join("'")
}

fn partial_splits(
    graph: &ParseGraph,
    table: &SyllableTable,
    input: &str,
    position: usize,
    prefix: &mut Vec<SyllableId>,
    output: &mut BTreeSet<String>,
) {
    for edge in graph.edges_from(position) {
        match &edge.kind {
            EdgeKind::Complete(id) => {
                prefix.push(*id);
                partial_splits(graph, table, input, edge.end, prefix, output);
                prefix.pop();
            }
            EdgeKind::Incomplete { .. } if !prefix.is_empty() => {
                output.insert(format!(
                    "{}'[{}]",
                    spelling(table, prefix),
                    &input[edge.start..edge.end]
                ));
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let parser = Parser::standard();
    let table = parser.table();
    let source = std::fs::read_to_string(&cli.characters).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("{}: {error}", cli.characters.display()),
        )
    })?;
    let imported = import_character_tsv(&source, table);
    if !imported.issues.is_empty() {
        return Err(format!("{}: {:?}", cli.characters.display(), imported.issues).into());
    }
    let mut rows = Vec::new();
    let mut complete_count = 0;
    let mut partial_count = 0;
    for (id, input) in table.iter() {
        let graph = parser.parse(input).unwrap();
        let complete = graph
            .complete_segmentations(usize::MAX)
            .iter()
            .filter(|path| path.len() > 1)
            .map(|path| spelling(table, path))
            .collect::<BTreeSet<_>>();
        let mut partial = BTreeSet::new();
        partial_splits(&graph, table, input, 0, &mut Vec::new(), &mut partial);
        complete_count += usize::from(!complete.is_empty());
        partial_count += usize::from(!partial.is_empty());
        if complete.is_empty() && partial.is_empty() {
            continue;
        }
        rows.push(format!(
            "{input}\t{}\t{}\t{}",
            imported.lexicon.exact(&[id]).len(),
            complete.into_iter().collect::<Vec<_>>().join("; "),
            partial.into_iter().collect::<Vec<_>>().join("; ")
        ));
    }
    println!(
        "# Development inventory generated from SyllableTable::standard(), Parser and characters.tsv."
    );
    println!(
        "# Regenerate: cargo run -p mirix-pinyin-core --example ambiguous_syllables --quiet -- {:?}",
        cli.characters
    );
    println!(
        "# A [tail] requires completion; plain syllables are complete. This is not a ranking allowlist."
    );
    println!(
        "# {} standard syllables; {} ambiguous; {} with complete splits; {} with partial-tail splits.",
        table.len(),
        rows.len(),
        complete_count,
        partial_count
    );
    println!("syllable\tcharacter_count\tcomplete_splits\tpartial_tail_splits");
    for row in rows {
        println!("{row}");
    }
    Ok(())
}
